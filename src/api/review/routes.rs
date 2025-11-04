use crate::api::models::AppState;
use crate::api::review::handlers::add_review_handler;
use axum::{routing::post, Router};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/reviews/add", post(add_review_handler))
}
