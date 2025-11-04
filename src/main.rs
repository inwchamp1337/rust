mod api;
mod config;
mod embedding;
mod storage;

use crate::api::{health_handler, AppState};
use crate::config::AppConfig;
use crate::embedding::EmbeddingService;
use crate::storage::{JsonlStorage, VectorIndex};
use axum::{
    routing::get,
    Router,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_http::trace::TraceLayer;
use tracing::{info, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .compact()
        .finish();
    
    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set tracing subscriber");

    info!("🚀 Starting Vector Search API Server");

    // Load configuration
    let config = AppConfig::load()?;
    info!("📋 Configuration loaded");
    info!("   - Index Type: {}", config.index.index_type);
    info!("   - Vector Dim: {}", config.index.vector_dim);
    info!("   - Server: {}:{}", config.server.host, config.server.port);

    // Initialize embedding service
    info!("🧠 Initializing embedding model...");
    let embedding_service = Arc::new(
        EmbeddingService::new(&config.embedding.model_name, config.embedding.max_length)?
    );
    info!("✅ Embedding model ready (dim: {})", embedding_service.dimension());

    // Initialize metadata storage
    info!("💾 Initializing metadata storage...");
    let metadata_store = Arc::new(JsonlStorage::new(&config.storage.metadata_path));
    metadata_store.initialize()?;
    let review_count = metadata_store.count_lines()?;
    info!("✅ Metadata storage ready ({} reviews)", review_count);

    // Initialize vector index
    info!("🔍 Initializing vector index...");
    let mut vector_index = VectorIndex::new(
        config.index.index_type.clone(),
        config.index.vector_dim,
        config.index.num_trees,
    );
    vector_index.initialize()?;
    
    // Try to load existing index
    if config.storage.index_path.exists() {
        info!("📂 Loading existing index from {:?}", config.storage.index_path);
        vector_index.load(&config.storage.index_path)?;
    }
    
    let vector_index = Arc::new(RwLock::new(vector_index));
    let index_path = config.storage.index_path.clone(); // Clone for shutdown handler
    info!("✅ Vector index ready");

    // Create application state
    let state = AppState {
        vector_index: vector_index.clone(),
        metadata_store,
        embedding_service,
    };

    // Build router with modular routes
    let app = Router::new()
        .route("/health", get(health_handler))
        .merge(api::review::routes())
        .merge(api::search::routes())
        .with_state(state)
        .layer(TraceLayer::new_for_http());

    // Start server
    let port = std::env::var("PORT").unwrap_or_else(|_| "8000".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    
    info!("🌐 Server listening on http://{}", addr);
    info!("");
    info!("📡 Available endpoints:");
    info!("   GET  /health           - Health check");
    info!("   POST /reviews/add      - Add new review");
    info!("   POST /reviews/search   - Search reviews");
    info!("");
    info!("✨ Server is ready to accept requests!");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Save index on graceful shutdown
    info!("💾 Saving vector index before shutdown...");
    if vector_index.read().await.save(&index_path).is_ok() {
        info!("✅ Index saved successfully");
    } else {
        info!("⚠️  Failed to save index");
    }

    info!("👋 Server shutting down gracefully");
    
    Ok(())
}

/// Graceful shutdown handler
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("🛑 Shutdown signal received");
}
