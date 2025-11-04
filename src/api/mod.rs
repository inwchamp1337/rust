pub mod models;
pub mod review;
pub mod search;

// Re-exports
pub use models::*;

// Health handler (simple, keep here)
use axum::{extract::State, Json};

pub async fn health_handler(State(state): State<AppState>) -> impl axum::response::IntoResponse {
    let total_reviews = state.metadata_store.count_lines().unwrap_or(0);
    Json(models::HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        total_reviews,
    })
}
