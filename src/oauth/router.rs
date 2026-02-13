//! OAuth Web Router
//!
//! Defines routes for OAuth web authentication

use std::sync::Arc;

use axum::{
    Router,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json,
};
use serde::Deserialize;

use super::handler::OAuthWebHandler;
use super::templates::{self, SELECT_PAGE_HTML};
use super::types::{ImportTokenRequest, ImportTokenResponse, RefreshResponse};

/// OAuth state for handlers
#[derive(Clone)]
pub struct OAuthState {
    pub handler: Arc<OAuthWebHandler>,
}

/// Create OAuth router
pub fn create_oauth_router(handler: Arc<OAuthWebHandler>) -> Router {
    let state = OAuthState { handler };

    Router::new()
        .route("/kiro", get(handle_select))
        .route("/kiro/start", get(handle_start))
        .route("/kiro/status", get(handle_status))
        .route("/kiro/import", post(handle_import))
        .route("/kiro/refresh", post(handle_refresh))
        .with_state(state)
}

/// Query parameters for start endpoint
#[derive(Debug, Deserialize)]
pub struct StartParams {
    method: String,
    #[serde(rename = "startUrl")]
    start_url: Option<String>,
    region: Option<String>,
}

/// Query parameters for status endpoint
#[derive(Debug, Deserialize)]
pub struct StatusParams {
    state: String,
}

/// Handle select page (GET /v0/oauth/kiro)
async fn handle_select() -> impl IntoResponse {
    Html(SELECT_PAGE_HTML)
}

/// Handle start authentication (GET /v0/oauth/kiro/start)
async fn handle_start(
    State(state): State<OAuthState>,
    Query(params): Query<StartParams>,
) -> Response {
    let result = match params.method.as_str() {
        "builder-id" => state.handler.start_builder_id_auth().await,
        "idc" => {
            let start_url = match params.start_url {
                Some(url) if !url.is_empty() => url,
                _ => {
                    return render_error("Missing startUrl parameter for IDC authentication");
                }
            };
            let region = params.region.as_deref().unwrap_or("us-east-1");
            state.handler.start_idc_auth(&start_url, region).await
        }
        _ => {
            return render_error(&format!("Unknown authentication method: {}", params.method));
        }
    };

    match result {
        Ok(session) => {
            let html = templates::render_start_page(
                &session.auth_url,
                &session.user_code,
                session.expires_in,
                &session.state_id,
            );
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(html.into())
                .unwrap()
        }
        Err(e) => render_error(&e),
    }
}

/// Handle status polling (GET /v0/oauth/kiro/status)
async fn handle_status(
    State(state): State<OAuthState>,
    Query(params): Query<StatusParams>,
) -> Response {
    match state.handler.get_status(&params.state) {
        Some(status) => Json(status).into_response(),
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "application/json")
            .body(r#"{"error": "Session not found"}"#.into())
            .unwrap(),
    }
}

/// Handle token import (POST /v0/oauth/kiro/import)
async fn handle_import(
    State(state): State<OAuthState>,
    Json(req): Json<ImportTokenRequest>,
) -> Json<ImportTokenResponse> {
    match state.handler.import_token(&req.refresh_token).await {
        Ok(resp) => Json(resp),
        Err(e) => Json(ImportTokenResponse {
            success: false,
            message: None,
            error: Some(e),
            file_name: None,
        }),
    }
}

/// Handle manual refresh (POST /v0/oauth/kiro/refresh)
async fn handle_refresh(State(state): State<OAuthState>) -> Json<RefreshResponse> {
    Json(state.handler.manual_refresh().await)
}

/// Render error page
fn render_error(error: &str) -> Response {
    let html = templates::render_error_page(error);
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(html.into())
        .unwrap()
}
