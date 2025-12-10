use std::sync::Arc;

use aws_config::SdkConfig;
use aws_lambda_events::event::eventbridge::EventBridgeEvent;
use docbox_database::{DatabasePoolCache, DatabasePoolCacheConfig};
use docbox_storage::StorageLayerFactory;
use lambda_runtime::{tracing, Error, LambdaEvent};
use tokio::sync::OnceCell;

static DEPENDENCIES: OnceCell<Dependencies> = OnceCell::new();

pub struct Dependencies {
    pub db_cache: Arc<DatabasePoolCache>,
    pub storage: StorageLayerFactory,
}

async fn dependencies() -> Result<Dependencies, Box<dyn Error>> {
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

    // Setup storage factory
    let storage_factory_config = StorageLayerFactoryConfig::from_env()?;
    let storage = StorageLayerFactory::from_config(&aws_config, storage_factory_config);

    Ok(Dependencies { db_cache, storage })
}

pub(crate) async fn outer_function_handler(
    event: LambdaEvent<EventBridgeEvent>,
) -> Result<(), Error> {
    let dependencies = DEPENDENCIES.get_or_try_init(dependencies).await?;
    function_handler(event, dependencies).await
}

async fn function_handler(
    event: LambdaEvent<EventBridgeEvent>,
    dependencies: &Dependencies,
) -> Result<(), Error> {
    // Run the presigned purge
    if let Err(error) = purge_expired_presigned_tasks(&dependencies.db, &dependencies.storage).await
    {
        tracing::error!(?error, "failed to purge presigned tasks");
    }

    Ok(())
}

/// Purge the presigned tasks for all tenants
#[tracing::instrument(skip_all)]
async fn purge_expired_presigned_tasks(
    db_cache: &'static Arc<DatabasePoolCache>,
    storage: &'static StorageLayerFactory,
) -> Result<(), PurgeExpiredPresignedError> {
    let db = db_cache.get_root_pool().await.map_err(|error| {
        tracing::error!(?error, "failed to connect to root database");
        PurgeExpiredPresignedError::ConnectDatabase
    })?;

    let tenants = Tenant::all(&db).await.map_err(|error| {
        tracing::error!(?error, "failed to query available tenants");
        PurgeExpiredPresignedError::QueryTenants
    })?;

    // Early drop the root database pool access
    drop(db);

    for tenant in tenants {
        // Create the database connection pool
        let db = db_cache.get_tenant_pool(&tenant).await.map_err(|error| {
            tracing::error!(?error, "failed to connect to tenant database");
            PurgeExpiredPresignedError::ConnectDatabase
        })?;

        let storage = storage.create_storage_layer(&tenant);

        if let Err(cause) = purge_expired_presigned_tasks_tenant(&db, &storage).await {
            tracing::error!(
                ?cause,
                ?tenant,
                "failed to purge presigned tasks for tenant"
            );
        }
    }

    Ok(())
}

/// Purge the presigned tasks for a specific tenant
async fn purge_expired_presigned_tasks_tenant(
    db: &DbPool,
    storage: &TenantStorageLayer,
) -> DbResult<()> {
    let current_date = Utc::now();
    let tasks = PresignedUploadTask::find_expired(db, current_date).await?;
    if tasks.is_empty() {
        return Ok(());
    }

    for task in tasks {
        // Delete the task itself
        if let Err(error) = PresignedUploadTask::delete(db, task.id).await {
            tracing::error!(?error, "failed to delete presigned upload task");
        }

        // Delete incomplete file uploads
        match task.status {
            PresignedTaskStatus::Completed { .. } => {
                // Upload completed, nothing to revert
            }
            PresignedTaskStatus::Failed { .. } | PresignedTaskStatus::Pending => {
                if let Err(error) = storage.delete_file(&task.file_key).await {
                    tracing::error!(
                        ?error,
                        "failed to delete expired presigned task file from tenant"
                    );
                }
            }
        }
    }

    Ok(())
}
