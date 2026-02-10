//! Admin API type definitions

use serde::{Deserialize, Serialize};

// ============ Credential Status ============

/// All credentials status response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsStatusResponse {
    /// Total number of credentials
    pub total: usize,
    /// Number of available credentials (not disabled)
    pub available: usize,
    /// Current active credential ID
    pub current_id: u64,
    /// List of credential statuses
    pub credentials: Vec<CredentialStatusItem>,
}

/// Status information for a single credential
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialStatusItem {
    /// Credential unique ID
    pub id: u64,
    /// Priority (lower number = higher priority)
    pub priority: u32,
    /// Whether disabled
    pub disabled: bool,
    /// Consecutive failure count
    pub failure_count: u32,
    /// Whether this is the current active credential
    pub is_current: bool,
    /// Token expiration time (RFC3339 format)
    pub expires_at: Option<String>,
    /// Authentication method
    pub auth_method: Option<String>,
    /// Whether has Profile ARN
    pub has_profile_arn: bool,
    /// SHA-256 hash of refreshToken (for frontend duplicate detection)
    pub refresh_token_hash: Option<String>,
    /// User email (for frontend display)
    pub email: Option<String>,
    /// API call success count
    pub success_count: u64,
    /// Last API call time (RFC3339 format)
    pub last_used_at: Option<String>,
}

// ============ Operation Requests ============

/// Enable/disable credential request
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetDisabledRequest {
    /// Whether to disable
    pub disabled: bool,
}

/// Modify priority request
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetPriorityRequest {
    /// New priority value
    pub priority: u32,
}

/// Add credential request
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCredentialRequest {
    /// Refresh token (required)
    pub refresh_token: String,

    /// Authentication method (optional, default social)
    #[serde(default = "default_auth_method")]
    pub auth_method: String,

    /// OIDC Client ID (required for IdC authentication)
    pub client_id: Option<String>,

    /// OIDC Client Secret (required for IdC authentication)
    pub client_secret: Option<String>,

    /// Priority (optional, default 0)
    #[serde(default)]
    pub priority: u32,

    /// Credential-level Region configuration (for OIDC token refresh)
    /// Falls back to global region in config.json if not configured
    pub region: Option<String>,

    /// Credential-level Auth Region (for Token refresh)
    pub auth_region: Option<String>,

    /// Credential-level API Region (for API requests)
    pub api_region: Option<String>,

    /// Credential-level Machine ID (optional, 64-character string)
    /// Falls back to machineId in config.json if not configured
    pub machine_id: Option<String>,

    /// User email (optional, for frontend display)
    pub email: Option<String>,
}

fn default_auth_method() -> String {
    "social".to_string()
}

/// Add credential success response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddCredentialResponse {
    pub success: bool,
    pub message: String,
    /// Newly added credential ID
    pub credential_id: u64,
    /// User email (if successfully obtained)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

// ============ Balance Query ============

/// Balance query response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BalanceResponse {
    /// Credential ID
    pub id: u64,
    /// User email
    pub email: Option<String>,
    /// Subscription type
    pub subscription_title: Option<String>,
    /// Current usage
    pub current_usage: f64,
    /// Usage limit
    pub usage_limit: f64,
    /// Remaining quota
    pub remaining: f64,
    /// Usage percentage
    pub usage_percentage: f64,
    /// Next reset time (Unix timestamp)
    pub next_reset_at: Option<f64>,
}

// ============ Load Balancing Configuration ============

/// Load balancing mode response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadBalancingModeResponse {
    /// Current mode ("priority" or "balanced")
    pub mode: String,
}

/// Set load balancing mode request
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetLoadBalancingModeRequest {
    /// Mode ("priority" or "balanced")
    pub mode: String,
}

// ============ Common Responses ============

/// Operation success response
#[derive(Debug, Serialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

impl SuccessResponse {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
        }
    }
}

/// Error response
#[derive(Debug, Serialize)]
pub struct AdminErrorResponse {
    pub error: AdminError,
}

#[derive(Debug, Serialize)]
pub struct AdminError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub message: String,
}

impl AdminErrorResponse {
    pub fn new(error_type: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: AdminError {
                error_type: error_type.into(),
                message: message.into(),
            },
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new("invalid_request", message)
    }

    pub fn authentication_error() -> Self {
        Self::new("authentication_error", "Invalid or missing admin API key")
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }

    pub fn api_error(message: impl Into<String>) -> Self {
        Self::new("api_error", message)
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new("internal_error", message)
    }
}
