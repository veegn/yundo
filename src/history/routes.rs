use super::db::{load_ranked_history_items, to_history_item};
use crate::common::AppState;
use axum::{extract::State, response::IntoResponse, Json};
use std::sync::Arc;

pub async fn history_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(
        load_ranked_history_items(&state.db, None)
            .await
            .into_iter()
            .map(to_history_item)
            .collect::<Vec<_>>(),
    )
}
