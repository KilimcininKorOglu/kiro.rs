//! OAuth Web Authentication Module
//!
//! Provides web-based OAuth authentication for Kiro (AWS CodeWhisperer)
//! Supports:
//! - AWS Builder ID (device code flow)
//! - AWS Identity Center (IDC) (device code flow)
//! - Token import from Kiro IDE
//! - Manual token refresh

mod handler;
mod router;
mod sso_oidc;
mod templates;
mod types;

pub use handler::OAuthWebHandler;
pub use router::create_oauth_router;
