use crate::api::models::*;
use crate::embedding::EmbeddingService;
use crate::storage::ReviewMetadata;
use axum::{extract::State, Json};
use tracing::{error, info};

pub async fn add_review_handler(
    State(state): State<AppState>,
    Json(request): Json<AddReviewRequest>,
) -> Result<Json<AddReviewResponse>, AppError> {
    // Validate
    request.validate().map_err(AppError::BadRequest)?;

    info!(product_id = %request.product_id, "Adding review");

    // Embed
    let text = EmbeddingService::prepare_review_text(&request.review_title, &request.review_body);
    let embedding = state
        .embedding_service
        .embed(&text)
        .map_err(|e| AppError::Internal(format!("Embedding failed: {}", e)))?;

    // Add to index & save
    let vector_id = {
        let mut index = state.vector_index.write().await;
        let id = index
            .add_vector(&embedding)
            .map_err(|e| AppError::Internal(format!("Add vector failed: {}", e)))?;
        
        index
            .save(&std::path::Path::new("data/reviews.index"))
            .map_err(|e| AppError::Internal(format!("Save index failed: {}", e)))?;
        
        id
    };

    // Store metadata
    let metadata = ReviewMetadata {
        review_title: request.review_title,
        review_body: request.review_body,
        product_id: request.product_id,
        review_rating: request.review_rating,
    };

    let stored_id = state
        .metadata_store
        .append(&metadata)
        .map_err(|e| AppError::Internal(format!("Store metadata failed: {}", e)))?;

    if vector_id != stored_id {
        error!(vector_id, stored_id, "ID mismatch");
    }

    info!(vector_id, "Review added");

    Ok(Json(AddReviewResponse {
        vector_id,
        status: "success".to_string(),
        message: format!("Review added with ID {}", vector_id),
    }))
}
