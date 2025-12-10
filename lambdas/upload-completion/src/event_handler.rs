use aws_lambda_events::event::s3::S3Event;
use docbox_core::events::EventPublisherFactory;
use docbox_processing::ProcessingLayer;
use lambda_runtime::{tracing, Error, LambdaEvent};
use tokio::sync::OnceCell;

static DEPENDENCIES: OnceCell<Dependencies> = OnceCell::new();

pub struct Dependencies {
    pub db_cache: Arc<DatabasePoolCache>,
    pub search: SearchIndexFactory,
    pub storage: StorageLayerFactory,
    pub events: EventPublisherFactory,
    pub processing: ProcessingLayer,
}

async fn dependencies() -> Result<Dependencies, Box<dyn Error>> {
    // Create the converter
    let converter_config = OfficeConverterConfig::from_env();
    let converter = OfficeConverter::from_config(converter_config)?;

    // Load the config for the processing layer
    let processing_layer_config = ProcessingLayerConfig::from_env()?;

    // Setup processing layer
    let processing = ProcessingLayer {
        office: OfficeProcessingLayer { converter },
        config: processing_layer_config,
    };

    let aws_config = aws_config().await;

    // Create secrets manager
    let secrets_config = SecretsManagerConfig::from_env()?;
    let secrets = SecretManager::from_config(&aws_config, secrets_config);

    // Load database credentials
    let mut db_pool_config = DatabasePoolCacheConfig::from_env()?;

    // Setup database cache / connector
    let db_cache = Arc::new(DatabasePoolCache::from_config(
        db_pool_config,
        secrets.clone(),
    ));

    // Create the SQS client
    // Warning: Will panic if the configuration provided is invalid
    let sqs_client = SqsClient::new(&aws_config);

    // Setup event publisher factories
    let sqs_publisher_factory = SqsEventPublisherFactory::new(sqs_client.clone());
    let events = EventPublisherFactory::new(sqs_publisher_factory);

    // Setup search index factory
    let search_config = SearchIndexFactoryConfig::from_env()?;
    let search =
        SearchIndexFactory::from_config(&aws_config, secrets, db_cache.clone(), search_config)?;

    // Setup storage factory
    let storage_factory_config = StorageLayerFactoryConfig::from_env()?;
    let storage = StorageLayerFactory::from_config(&aws_config, storage_factory_config);

    Ok(Dependencies {
        db_cache,
        storage,
        processing,
        events,
        search,
    })
}

pub(crate) async fn outer_function_handler(
    event: LambdaEvent<EventBridgeEvent>,
) -> Result<(), Error> {
    let dependencies = DEPENDENCIES.get_or_try_init(dependencies).await?;
    function_handler(event, dependencies).await
}

async fn function_handler(
    event: LambdaEvent<S3Event>,
    dependencies: &Dependencies,
) -> Result<(), Error> {
    // Extract some useful information from the request
    let payload = event.payload;
    tracing::info!("Payload: {:?}", payload);

    // TODO: Replace unwraps with errors
    let record = payload.records.first().unwrap();
    let bucket = record.s3.bucket;
    let object = record.s3.object;

    let bucket_name = bucket.name.unwrap();
    let object_key = object.key.unwrap();

    handle_file_uploaded(dependencies, bucket_name, object_key).await;

    Ok(())
}

/// Handle file upload notifications
#[tracing::instrument(skip(data))]
pub async fn handle_file_uploaded(data: &Dependencies, bucket_name: String, object_key: String) {
    let tenant = {
        let db = match data.db_cache.get_root_pool().await {
            Ok(value) => value,
            Err(error) => {
                tracing::error!(?error, "failed to acquire root database pool");
                return;
            }
        };

        match Tenant::find_by_bucket(&db, &bucket_name).await {
            Ok(Some(value)) => value,
            Ok(None) => {
                tracing::warn!(
                    "file was uploaded into a bucket sqs is listening to but there was no matching tenant"
                );
                return;
            }
            Err(error) => {
                tracing::error!(?error, "failed to query tenant for bucket");
                return;
            }
        }
    };

    // Provide a span that contains the tenant metadata
    let span = tracing::info_span!("tenant", tenant_id = %tenant.id, tenant_env = %tenant.env);

    handle_file_uploaded_tenant(tenant, data, bucket_name, object_key)
        .instrument(span)
        .await;
}

/// Handle file upload notification once the tenant has been identified
#[tracing::instrument(skip(data))]
pub async fn handle_file_uploaded_tenant(
    tenant: Tenant,
    data: &Dependencies,
    bucket_name: String,
    object_key: String,
) {
    let object_key = match urlencoding::decode(&object_key) {
        Ok(value) => value.to_string(),
        Err(error) => {
            tracing::warn!(
                ?error,
                "file was uploaded into a bucket but had an invalid file name"
            );
            return;
        }
    };

    let db = match data.db_cache.get_tenant_pool(&tenant).await {
        Ok(value) => value,
        Err(error) => {
            tracing::error!(?error, "failed to get tenant database pool");
            return;
        }
    };

    // Locate a pending upload task for the uploaded file
    let task = match PresignedUploadTask::find_by_file_key(&db, &object_key).await {
        Ok(Some(task)) => task,
        // Ignore files that aren't attached to a presigned upload task
        // (Things like generated files will show up here)
        Ok(None) => {
            tracing::debug!("uploaded file was not a presigned upload");
            return;
        }
        Err(error) => {
            tracing::error!(?error, "unable to query presigned upload");
            return;
        }
    };

    let scope = task.document_box.clone();

    // Retrieve the target folder
    let folder = match Folder::find_by_id(&db, &scope, task.folder_id).await {
        Ok(Some(value)) => value,
        Ok(None) => {
            tracing::error!("presigned upload folder no longer exists");
            return;
        }
        Err(error) => {
            tracing::error!(?error, "unable to query folder");
            return;
        }
    };

    // Update stored editing user data
    let complete = CompletePresigned { task, folder };

    let search = data.search.create_search_index(&tenant);
    let storage = data.storage.create_storage_layer(&tenant);
    let events = data.events.create_event_publisher(&tenant);

    // Create task future that performs the file upload
    if let Err(error) =
        safe_complete_presigned(db, search, storage, events, data.processing, complete).await
    {
        tracing::error!(?error, "failed to complete presigned file upload");
    }
}
