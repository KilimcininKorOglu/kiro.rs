//! Admin API error type definitions

use std::fmt;

use axum::http::StatusCode;

use super::types::AdminErrorResponse;

/// Admin service error types
#[derive(Debug)]
pub enum AdminServiceError {
    /// Credential not found
    NotFound { id: u64 },

    /// Credential not found (by ID)
    CredentialNotFound(u64),

    /// Upstream service call failed (network, API errors, etc.)
    UpstreamError(String),

    /// Internal state error
    InternalError(String),

    /// Invalid credential (validation failed)
    InvalidCredential(String),

    /// Token refresh failed
    TokenRefreshFailed(String),
}

impl fmt::Display for AdminServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdminServiceError::NotFound { id } => {
                write!(f, "Credential not found: {}", id)
            }
            AdminServiceError::CredentialNotFound(id) => {
                write!(f, "Credential not found: {}", id)
            }
            AdminServiceError::UpstreamError(msg) => write!(f, "Upstream service error: {}", msg),
            AdminServiceError::InternalError(msg) => write!(f, "Internal error: {}", msg),
            AdminServiceError::InvalidCredential(msg) => write!(f, "Invalid credential: {}", msg),
            AdminServiceError::TokenRefreshFailed(msg) => write!(f, "Token refresh failed: {}", msg),
        }
    }
}

impl std::error::Error for AdminServiceError {}

impl AdminServiceError {
    /// Get corresponding HTTP status code
    pub fn status_code(&self) -> StatusCode {
        match self {
            AdminServiceError::NotFound { .. } => StatusCode::NOT_FOUND,
            AdminServiceError::CredentialNotFound(_) => StatusCode::NOT_FOUND,
            AdminServiceError::UpstreamError(_) => StatusCode::BAD_GATEWAY,
            AdminServiceError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
            AdminServiceError::InvalidCredential(_) => StatusCode::BAD_REQUEST,
            AdminServiceError::TokenRefreshFailed(_) => StatusCode::BAD_GATEWAY,
        }
    }

    /// Convert to API error response
    pub fn into_response(self) -> AdminErrorResponse {
        match &self {
            AdminServiceError::NotFound { .. } => AdminErrorResponse::not_found(self.to_string()),
            AdminServiceError::CredentialNotFound(_) => AdminErrorResponse::not_found(self.to_string()),
            AdminServiceError::UpstreamError(_) => AdminErrorResponse::api_error(self.to_string()),
            AdminServiceError::InternalError(_) => {
                AdminErrorResponse::internal_error(self.to_string())
            }
            AdminServiceError::InvalidCredential(_) => {
                AdminErrorResponse::invalid_request(self.to_string())
            }
            AdminServiceError::TokenRefreshFailed(_) => {
                AdminErrorResponse::api_error(self.to_string())
            }
        }
    }
}
