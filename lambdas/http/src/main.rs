use crate::{
    extensions::max_file_size::MaxFileSizeBytes, middleware::api_key::ApiKeyLayer, routes::router,
};
use axum::{Extension, Router};
use docbox_core::{
    aws::{SqsClient, aws_config},
    events::{EventPublisherFactory, sqs::SqsEventPublisherFactory},
    tenant::tenant_cache::TenantCache,
};
use docbox_database::{DatabasePoolCache, DatabasePoolCacheConfig};
use docbox_search::{SearchIndexFactory, SearchIndexFactoryConfig};
use docbox_secrets::{SecretManager, SecretsManagerConfig};
use docbox_storage::{StorageLayerFactory, StorageLayerFactoryConfig};
use docbox_web_scraper::{WebsiteMetaService, WebsiteMetaServiceConfig};
use lambda_http::{Error, run_with_streaming_response, tracing};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

pub mod docs;
mod error;
mod extensions;
mod middleware;
mod models;
mod routes;

/// The server version extracted from the Cargo.toml
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<(), Error> {
    #[cfg(debug_assertions)]
    {
        _ = dotenvy::dotenv();
    }

    tracing::init_default_subscriber();

    let app = app().await?;

    run_with_streaming_response(app).await
}

// TODO: Needs a db_cache.close_all() cleanup logic when the program exits
async fn app() -> Result<Router, Box<dyn std::error::Error + Send + Sync>> {
    let max_file_size_bytes = match std::env::var("DOCBOX_MAX_FILE_SIZE_BYTES") {
        Ok(value) => value.parse::<i32>()?,
        // Default max file size in bytes (100MB)
        Err(_) => 100 * 1000 * 1024,
    };

    // Create website scraping service
    let website_meta_service_config = WebsiteMetaServiceConfig::from_env()?;
    let website_meta_service = Arc::new(WebsiteMetaService::from_config(
        website_meta_service_config,
    )?);

    let aws_config = aws_config().await;

    // Create secrets manager
    let secrets_config = SecretsManagerConfig::from_env()?;
    let secrets = SecretManager::from_config(&aws_config, secrets_config);

    // Load database credentials
    let db_pool_config = DatabasePoolCacheConfig::from_env()?;

    // API key
    let api_key = std::env::var("DOCBOX_API_KEY").ok();

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

    // Create tenant cache
    let tenant_cache = Arc::new(TenantCache::new());

    // Setup router
    let mut app = router()
        .layer(Extension(search))
        .layer(Extension(storage))
        .layer(Extension(db_cache.clone()))
        .layer(Extension(website_meta_service))
        .layer(Extension(events))
        .layer(Extension(tenant_cache))
        .layer(Extension(MaxFileSizeBytes(max_file_size_bytes)))
        .layer(TraceLayer::new_for_http());

    if let Some(api_key) = api_key {
        app = app.layer(ApiKeyLayer::new(api_key));
    } else {
        tracing::warn!(
            "DOCBOX_API_KEY not specified, its recommended you set one for security reasons"
        )
    }

    // Development mode CORS access for local browser testing
    #[cfg(debug_assertions)]
    let app = app.layer(tower_http::cors::CorsLayer::very_permissive());

    Ok(app)
}
