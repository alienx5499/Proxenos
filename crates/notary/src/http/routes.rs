//! The Axum router.

use std::sync::Arc;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware::map_response;
use axum::routing::{get, post};

use super::{AppState, HttpPorts, deals, handlers, seal_page};

/// Every request body the API takes is a small JSON object.
pub const BODY_LIMIT: usize = 16 * 1024;

pub fn router<P: HttpPorts>(state: Arc<AppState<P>>) -> Router {
    Router::new()
        .route("/health", get(handlers::health))
        .route("/seal/{token}", get(seal_page::page))
        .route("/static/seal.js", get(seal_page::script))
        .route("/static/seal.css", get(seal_page::style))
        .route(
            "/api/seal/{token}",
            get(handlers::seal_form::<P>)
                .post(handlers::seal::<P>)
                .layer(map_response(seal_page::no_store)),
        )
        .route("/api/deals", post(deals::open::<P>))
        .route("/api/deals/{id}/admit", post(deals::admit::<P>))
        .route("/api/deals/{id}/timeline", get(handlers::timeline::<P>))
        .route("/api/deals/{id}/receipt", get(handlers::receipt::<P>))
        .route("/api/notary/key", get(handlers::notary_key::<P>))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state)
}
