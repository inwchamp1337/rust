use crate::api::models::*;
use crate::embedding::EmbeddingService;
use crate::storage::{JsonlStorage, ReviewMetadata, VectorIndex};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub vector_index: Arc<RwLock<VectorIndex>>,
    pub metadata_store: Arc<JsonlStorage>,
    pub embedding_service: Arc<EmbeddingService>,
}

/// Health check endpoint
pub async fn health_handler(State(state): State<AppState>) -> impl IntoResponse {
    let total_reviews = state
        .metadata_store
        .count_lines()
        .unwrap_or(0);

    Json(HealthResponse {
        status: "healthy".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        total_reviews,
    })
}

/// Add a new review
pub async fn add_review_handler(
    State(state): State<AppState>,
    Json(request): Json<AddReviewRequest>,
) -> Result<Json<AddReviewResponse>, AppError> {
    // Validate request
    request.validate()
        .map_err(|e| AppError::BadRequest(e))?;

    info!(
        product_id = %request.product_id,
        rating = request.review_rating,
        "Processing add review request"
    );

    // Prepare text for embedding
    let text = EmbeddingService::prepare_review_text(
        &request.review_title,
        &request.review_body,
    );

    // Generate embedding
    let embedding = state
        .embedding_service
        .embed(&text)
        .map_err(|e| AppError::Internal(format!("Failed to generate embedding: {}", e)))?;

    info!(
        embedding_dim = embedding.len(),
        "Generated embedding vector"
    );

    // Add vector to index
    let vector_id = {
        let mut index = state.vector_index.write().await;
        let id = index
            .add_vector(&embedding)
            .map_err(|e| AppError::Internal(format!("Failed to add vector to index: {}", e)))?;
        
        // Save index after adding vector (append-only)
        index
            .save(&std::path::Path::new("data/reviews.index"))
            .map_err(|e| AppError::Internal(format!("Failed to save index: {}", e)))?;
        
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
        .map_err(|e| AppError::Internal(format!("Failed to store metadata: {}", e)))?;

    // Verify IDs match
    if vector_id != stored_id {
        error!(
            vector_id = vector_id,
            stored_id = stored_id,
            "Vector ID mismatch!"
        );
    }

    info!(
        vector_id = vector_id,
        "Successfully added review and saved index"
    );

    Ok(Json(AddReviewResponse {
        vector_id,
        status: "success".to_string(),
        message: format!("Review added with ID {}", vector_id),
    }))
}

/// Search for similar reviews
pub async fn search_handler(
    State(state): State<AppState>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, AppError> {
    // Validate request
    request.validate()
        .map_err(|e| AppError::BadRequest(e))?;

    info!(
        query = %request.query,
        top_k = request.top_k,
        "Processing search request"
    );

    // Generate query embedding
    let query_embedding = state
        .embedding_service
        .embed(&request.query)
        .map_err(|e| AppError::Internal(format!("Failed to generate query embedding: {}", e)))?;

    // Search in vector index
    let search_results = state
        .vector_index
        .read()
        .await
        .search(&query_embedding, request.top_k)
        .map_err(|e| AppError::Internal(format!("Failed to search index: {}", e)))?;

    info!(
        results_found = search_results.len(),
        "Vector search completed"
    );

    // Retrieve metadata for results
    let vector_ids: Vec<usize> = search_results
        .iter()
        .map(|r| r.vector_id)
        .collect();

    let metadata_list = state
        .metadata_store
        .read_batch(&vector_ids)
        .map_err(|e| AppError::Internal(format!("Failed to read metadata: {}", e)))?;

    // Combine results
    let results: Vec<SearchResultItem> = search_results
        .iter()
        .zip(metadata_list.iter())
        .map(|(search_result, metadata)| SearchResultItem {
            review_title: metadata.review_title.clone(),
            review_body: metadata.review_body.clone(),
            product_id: metadata.product_id.clone(),
            review_rating: metadata.review_rating,
            similarity_score: 1.0 - search_result.distance, // Convert distance to similarity
            vector_id: search_result.vector_id,
        })
        .collect();

    Ok(Json(SearchResponse {
        total_found: results.len(),
        results,
        query: request.query,
    }))
}

/// Custom error type
#[derive(Debug)]
pub enum AppError {
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Internal(msg) => {
                error!("Internal error: {}", msg);
                (StatusCode::INTERNAL_SERVER_ERROR, msg)
            }
        };

        let body = Json(ErrorResponse {
            error: status.to_string(),
            message,
        });

        (status, body).into_response()
    }
}
