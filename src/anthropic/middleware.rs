//! Anthropic API middleware

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Json, Response},
};

use crate::common::auth;
use crate::kiro::provider::KiroProvider;

use super::types::ErrorResponse;

/// Application shared state
#[derive(Clone)]
pub struct AppState {
    /// API key
    pub api_key: String,
    /// Kiro Provider (optional, used for actual API calls)
    /// Internally uses MultiTokenManager, already supports thread-safe multi-credential management
    pub kiro_provider: Option<Arc<KiroProvider>>,
    /// Profile ARN (optional, used for requests)
    pub profile_arn: Option<String>,
}

impl AppState {
    /// Create new application state
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            kiro_provider: None,
            profile_arn: None,
        }
    }

    /// Set KiroProvider
    pub fn with_kiro_provider(mut self, provider: KiroProvider) -> Self {
        self.kiro_provider = Some(Arc::new(provider));
        self
    }

    /// Set Profile ARN
    pub fn with_profile_arn(mut self, arn: impl Into<String>) -> Self {
        self.profile_arn = Some(arn.into());
        self
    }
}

/// API Key authentication middleware
pub async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    match auth::extract_api_key(&request) {
        Some(key) if auth::constant_time_eq(&key, &state.api_key) => next.run(request).await,
        _ => {
            let error = ErrorResponse::authentication_error();
            (StatusCode::UNAUTHORIZED, Json(error)).into_response()
        }
    }
}

/// CORS middleware layer
///
/// **Security note**: Current configuration allows all origins (Any), this is to support public API services.
/// If stricter security control is needed, please configure specific allowed origins, methods and headers according to actual requirements.
///
/// # Configuration notes
/// - `allow_origin(Any)`: Allow requests from any origin
/// - `allow_methods(Any)`: Allow any HTTP method
/// - `allow_headers(Any)`: Allow any request header
pub fn cors_layer() -> tower_http::cors::CorsLayer {
    use tower_http::cors::{Any, CorsLayer};

    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any)
}
