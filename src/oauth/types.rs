//! OAuth Web Authentication Types

use serde::{Deserialize, Serialize};

/// Authentication session status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthSessionStatus {
    Pending,
    Success,
    Failed,
}

/// Web authentication session
#[derive(Debug, Clone)]
pub struct WebAuthSession {
    pub state_id: String,
    pub device_code: String,
    pub user_code: String,
    pub auth_url: String,
    pub verification_uri: String,
    pub expires_in: i64,
    pub interval: i64,
    pub status: AuthSessionStatus,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub error: Option<String>,
    pub auth_method: String,
    pub start_url: Option<String>,
    pub region: String,
    pub client_id: String,
    pub client_secret: String,
}

/// Status response for polling
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub status: AuthSessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Import token request
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportTokenRequest {
    pub refresh_token: String,
}

/// Import token response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportTokenResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
}

/// Manual refresh response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshResponse {
    pub success: bool,
    pub message: String,
    pub refreshed_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warnings: Option<Vec<String>>,
}

/// SSO OIDC Register Client Response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterClientResponse {
    pub client_id: String,
    pub client_secret: String,
    pub client_id_issued_at: Option<i64>,
    pub client_secret_expires_at: Option<i64>,
}

/// SSO OIDC Start Device Authorization Response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartDeviceAuthResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i64,
    pub interval: Option<i64>,
}

/// SSO OIDC Create Token Response
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTokenResponse {
    pub access_token: String,
    pub token_type: Option<String>,
    pub expires_in: Option<i64>,
    pub refresh_token: Option<String>,
}

/// SSO OIDC Error Response
#[derive(Debug, Deserialize)]
pub struct OidcErrorResponse {
    pub error: String,
    pub error_description: Option<String>,
}
