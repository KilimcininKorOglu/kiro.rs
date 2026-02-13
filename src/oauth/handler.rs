//! OAuth Web Handler
//!
//! Manages OAuth sessions and handles authentication flow

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{Duration, Utc};
use parking_lot::Mutex;

use crate::http_client::ProxyConfig;
use crate::kiro::model::credentials::KiroCredentials;
use crate::kiro::token_manager::MultiTokenManager;
use crate::model::config::Config;

use super::sso_oidc::{CreateTokenResult, SsoOidcClient};
use super::types::*;

/// OAuth Web Handler
pub struct OAuthWebHandler {
    config: Config,
    proxy: Option<ProxyConfig>,
    sessions: Arc<Mutex<HashMap<String, WebAuthSession>>>,
    token_manager: Arc<MultiTokenManager>,
}

impl OAuthWebHandler {
    pub fn new(
        config: Config,
        proxy: Option<ProxyConfig>,
        token_manager: Arc<MultiTokenManager>,
    ) -> Self {
        Self {
            config,
            proxy,
            sessions: Arc::new(Mutex::new(HashMap::new())),
            token_manager,
        }
    }

    /// Generate a random state ID
    fn generate_state_id() -> String {
        use base64::Engine;
        let mut bytes = [0u8; 16];
        for byte in &mut bytes {
            *byte = fastrand::u8(..);
        }
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }

    /// Start Builder ID authentication
    pub async fn start_builder_id_auth(&self) -> Result<WebAuthSession, String> {
        let region = SsoOidcClient::default_region();
        let start_url = SsoOidcClient::builder_id_start_url();
        self.start_device_auth("builder-id", start_url, region).await
    }

    /// Start IDC authentication
    pub async fn start_idc_auth(&self, start_url: &str, region: &str) -> Result<WebAuthSession, String> {
        self.start_device_auth("idc", start_url, region).await
    }

    /// Start device code authentication flow
    async fn start_device_auth(
        &self,
        auth_method: &str,
        start_url: &str,
        region: &str,
    ) -> Result<WebAuthSession, String> {
        let state_id = Self::generate_state_id();
        let sso_client = SsoOidcClient::new(self.proxy.clone(), self.config.tls_backend);

        // Register client
        let reg_resp = sso_client
            .register_client(region)
            .await
            .map_err(|e| format!("Failed to register client: {}", e))?;

        // Start device authorization
        let auth_resp = sso_client
            .start_device_authorization(
                &reg_resp.client_id,
                &reg_resp.client_secret,
                start_url,
                region,
            )
            .await
            .map_err(|e| format!("Failed to start device authorization: {}", e))?;

        let session = WebAuthSession {
            state_id: state_id.clone(),
            device_code: auth_resp.device_code,
            user_code: auth_resp.user_code,
            auth_url: auth_resp.verification_uri_complete,
            verification_uri: auth_resp.verification_uri,
            expires_in: auth_resp.expires_in,
            interval: auth_resp.interval.unwrap_or(5),
            status: AuthSessionStatus::Pending,
            started_at: Utc::now(),
            completed_at: None,
            expires_at: None,
            error: None,
            auth_method: auth_method.to_string(),
            start_url: if auth_method == "idc" {
                Some(start_url.to_string())
            } else {
                None
            },
            region: region.to_string(),
            client_id: reg_resp.client_id,
            client_secret: reg_resp.client_secret,
        };

        // Store session
        {
            let mut sessions = self.sessions.lock();
            sessions.insert(state_id.clone(), session.clone());
        }

        // Start polling in background
        self.start_polling(state_id);

        Ok(session)
    }

    /// Start background polling for token
    fn start_polling(&self, state_id: String) {
        let sessions = self.sessions.clone();
        let proxy = self.proxy.clone();
        let tls_backend = self.config.tls_backend;
        let token_manager = self.token_manager.clone();

        tokio::spawn(async move {
            let session_data = {
                let sessions = sessions.lock();
                sessions.get(&state_id).cloned()
            };

            let session = match session_data {
                Some(s) => s,
                None => return,
            };

            let sso_client = SsoOidcClient::new(proxy, tls_backend);
            let mut interval = std::time::Duration::from_secs(session.interval as u64);
            let deadline = session.started_at + Duration::seconds(session.expires_in);

            loop {
                tokio::time::sleep(interval).await;

                if Utc::now() >= deadline {
                    let mut sessions = sessions.lock();
                    if let Some(s) = sessions.get_mut(&state_id) {
                        s.status = AuthSessionStatus::Failed;
                        s.error = Some("Authentication timed out".to_string());
                        s.completed_at = Some(Utc::now());
                    }
                    break;
                }

                let result = sso_client
                    .create_token(
                        &session.client_id,
                        &session.client_secret,
                        &session.device_code,
                        &session.region,
                    )
                    .await;

                match result {
                    Ok(CreateTokenResult::Success(token_resp)) => {
                        let expires_in = token_resp.expires_in.unwrap_or(3600);
                        let expires_at = Utc::now() + Duration::seconds(expires_in);

                        // Fetch profile ARN
                        let profile_arn = sso_client
                            .fetch_profile_arn(&token_resp.access_token, &session.region)
                            .await;

                        // Create credentials
                        let mut credentials = KiroCredentials::default();
                        credentials.access_token = Some(token_resp.access_token);
                        credentials.refresh_token = token_resp.refresh_token;
                        credentials.profile_arn = profile_arn;
                        credentials.expires_at = Some(expires_at.to_rfc3339());
                        credentials.auth_method = Some(session.auth_method.clone());
                        credentials.client_id = Some(session.client_id.clone());
                        credentials.client_secret = Some(session.client_secret.clone());
                        credentials.region = Some(session.region.clone());

                        // Add to token manager
                        if let Err(e) = token_manager.add_credential(credentials).await {
                            tracing::error!("Failed to add credential: {}", e);
                        }

                        // Update session
                        let mut sessions = sessions.lock();
                        if let Some(s) = sessions.get_mut(&state_id) {
                            s.status = AuthSessionStatus::Success;
                            s.completed_at = Some(Utc::now());
                            s.expires_at = Some(expires_at);
                        }

                        tracing::info!("OAuth Web: authentication successful");
                        break;
                    }
                    Ok(CreateTokenResult::Pending) => {
                        // Continue polling
                        continue;
                    }
                    Ok(CreateTokenResult::SlowDown) => {
                        interval += std::time::Duration::from_secs(5);
                        continue;
                    }
                    Ok(CreateTokenResult::Expired) => {
                        let mut sessions = sessions.lock();
                        if let Some(s) = sessions.get_mut(&state_id) {
                            s.status = AuthSessionStatus::Failed;
                            s.error = Some("Device code expired".to_string());
                            s.completed_at = Some(Utc::now());
                        }
                        break;
                    }
                    Err(e) => {
                        let mut sessions = sessions.lock();
                        if let Some(s) = sessions.get_mut(&state_id) {
                            s.status = AuthSessionStatus::Failed;
                            s.error = Some(format!("Token creation failed: {}", e));
                            s.completed_at = Some(Utc::now());
                        }
                        tracing::error!("OAuth Web: token polling failed: {}", e);
                        break;
                    }
                }
            }
        });
    }

    /// Get session by state ID
    pub fn get_session(&self, state_id: &str) -> Option<WebAuthSession> {
        let sessions = self.sessions.lock();
        sessions.get(state_id).cloned()
    }

    /// Get session status
    pub fn get_status(&self, state_id: &str) -> Option<StatusResponse> {
        let sessions = self.sessions.lock();
        let session = sessions.get(state_id)?;

        let mut response = StatusResponse {
            status: session.status,
            remaining_seconds: None,
            completed_at: None,
            expires_at: None,
            error: None,
        };

        match session.status {
            AuthSessionStatus::Pending => {
                let elapsed = (Utc::now() - session.started_at).num_seconds();
                let remaining = session.expires_in - elapsed;
                response.remaining_seconds = Some(remaining.max(0));
            }
            AuthSessionStatus::Success => {
                response.completed_at = session.completed_at.map(|t| t.to_rfc3339());
                response.expires_at = session.expires_at.map(|t| t.to_rfc3339());
            }
            AuthSessionStatus::Failed => {
                response.error = session.error.clone();
                response.completed_at = session.completed_at.map(|t| t.to_rfc3339());
            }
        }

        Some(response)
    }

    /// Import token from refresh token
    pub async fn import_token(&self, refresh_token: &str) -> Result<ImportTokenResponse, String> {
        let refresh_token = refresh_token.trim();

        if refresh_token.is_empty() {
            return Ok(ImportTokenResponse {
                success: false,
                message: None,
                error: Some("Refresh token is required".to_string()),
                file_name: None,
            });
        }

        // Validate token format
        if !refresh_token.starts_with("aorAAAAAG") {
            return Ok(ImportTokenResponse {
                success: false,
                message: None,
                error: Some("Invalid token format. Token should start with aorAAAAAG...".to_string()),
                file_name: None,
            });
        }

        // Create credentials with refresh token
        let mut credentials = KiroCredentials::default();
        credentials.refresh_token = Some(refresh_token.to_string());
        credentials.auth_method = Some("social".to_string());

        // Add to token manager (will trigger refresh)
        match self.token_manager.add_credential(credentials).await {
            Ok(_) => Ok(ImportTokenResponse {
                success: true,
                message: Some("Token imported successfully".to_string()),
                error: None,
                file_name: Some("credentials.json".to_string()),
            }),
            Err(e) => Ok(ImportTokenResponse {
                success: false,
                message: None,
                error: Some(format!("Failed to import token: {}", e)),
                file_name: None,
            }),
        }
    }

    /// Manual refresh all tokens
    pub async fn manual_refresh(&self) -> RefreshResponse {
        match self.token_manager.refresh_all_tokens().await {
            Ok(count) => RefreshResponse {
                success: true,
                message: format!("Refreshed {} token(s)", count),
                refreshed_count: count,
                warnings: None,
            },
            Err(e) => RefreshResponse {
                success: false,
                message: format!("Refresh failed: {}", e),
                refreshed_count: 0,
                warnings: None,
            },
        }
    }

    /// Cleanup expired sessions
    pub fn cleanup_expired_sessions(&self) {
        let mut sessions = self.sessions.lock();
        let now = Utc::now();

        sessions.retain(|_, session| {
            // Keep pending sessions that haven't expired
            if session.status == AuthSessionStatus::Pending {
                let deadline = session.started_at + Duration::seconds(session.expires_in);
                return now < deadline;
            }

            // Keep completed sessions for 30 minutes
            if let Some(completed_at) = session.completed_at {
                return now < completed_at + Duration::minutes(30);
            }

            false
        });
    }
}
