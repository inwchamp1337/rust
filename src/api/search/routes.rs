use crate::api::models::AppState;
use crate::api::search::handlers::search_handler;
use axum::{routing::post, Router};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/reviews/search", post(search_handler))
}
