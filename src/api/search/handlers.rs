use crate::api::models::*;
use axum::{extract::State, Json};
use tracing::info;

pub async fn search_handler(
    State(state): State<AppState>,
    Json(request): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, AppError> {
    // Validate
    request.validate().map_err(AppError::BadRequest)?;

    info!(query = %request.query, k = request.top_k, "Searching");

    // Embed query
    let embedding = state
        .embedding_service
        .embed(&request.query)
        .map_err(|e| AppError::Internal(format!("Embedding failed: {}", e)))?;

    // Search
    let search_results = state
        .vector_index
        .read()
        .await
        .search(&embedding, request.top_k)
        .map_err(|e| AppError::Internal(format!("Search failed: {}", e)))?;

    info!(found = search_results.len(), "Search complete");

    // Get metadata
    let vector_ids: Vec<usize> = search_results.iter().map(|r| r.vector_id).collect();
    let metadata_list = state
        .metadata_store
        .read_batch(&vector_ids)
        .map_err(|e| AppError::Internal(format!("Metadata read failed: {}", e)))?;

    // Combine results
    let results: Vec<SearchResultItem> = search_results
        .iter()
        .zip(metadata_list.iter())
        .map(|(sr, meta)| SearchResultItem {
            review_title: meta.review_title.clone(),
            review_body: meta.review_body.clone(),
            product_id: meta.product_id.clone(),
            review_rating: meta.review_rating,
            similarity_score: 1.0 - sr.distance,
            vector_id: sr.vector_id,
        })
        .collect();

    let total = results.len();

    Ok(Json(SearchResponse {
        query: request.query,
        results,
        total_found: total,
    }))
}
