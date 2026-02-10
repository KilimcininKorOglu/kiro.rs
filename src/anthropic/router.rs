//! Anthropic API routing configuration

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};

use crate::kiro::provider::KiroProvider;

use super::{
    handlers::{count_tokens, get_models, post_messages, post_messages_cc},
    middleware::{AppState, auth_middleware, cors_layer},
};

/// Maximum request body size limit (50MB)
const MAX_BODY_SIZE: usize = 50 * 1024 * 1024;

/// Create Anthropic API router
///
/// # Endpoints
/// - `GET /v1/models` - Get list of available models
/// - `POST /v1/messages` - Create message (conversation)
/// - `POST /v1/messages/count_tokens` - Calculate token count
///
/// # Authentication
/// All `/v1` paths require API Key authentication, supporting:
/// - `x-api-key` header
/// - `Authorization: Bearer <token>` header
///
/// # Parameters
/// - `api_key`: API key for validating client requests
/// - `kiro_provider`: Optional KiroProvider for calling upstream API

/// Create Anthropic API router with KiroProvider
pub fn create_router_with_provider(
    api_key: impl Into<String>,
    kiro_provider: Option<KiroProvider>,
    profile_arn: Option<String>,
) -> Router {
    let mut state = AppState::new(api_key);
    if let Some(provider) = kiro_provider {
        state = state.with_kiro_provider(provider);
    }
    if let Some(arn) = profile_arn {
        state = state.with_profile_arn(arn);
    }

    // Authenticated /v1 routes
    let v1_routes = Router::new()
        .route("/models", get(get_models))
        .route("/messages", post(post_messages))
        .route("/messages/count_tokens", post(count_tokens))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Authenticated /cc/v1 routes (Claude Code compatible endpoints)
    // Difference from /v1: streaming response waits for contextUsageEvent before sending message_start
    let cc_v1_routes = Router::new()
        .route("/messages", post(post_messages_cc))
        .route("/messages/count_tokens", post(count_tokens))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    Router::new()
        .nest("/v1", v1_routes)
        .nest("/cc/v1", cc_v1_routes)
        .layer(cors_layer())
        .layer(DefaultBodyLimit::max(MAX_BODY_SIZE))
        .with_state(state)
}
