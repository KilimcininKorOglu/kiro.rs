//! AWS SSO OIDC Client
//!
//! Handles AWS SSO OIDC authentication for Builder ID and Identity Center (IDC)

use anyhow::{bail, Result};
use serde_json::json;

use crate::http_client::{ProxyConfig, build_client};
use crate::model::config::TlsBackend;

use super::types::{
    CreateTokenResponse, OidcErrorResponse, RegisterClientResponse, StartDeviceAuthResponse,
};

const DEFAULT_REGION: &str = "us-east-1";
const BUILDER_ID_START_URL: &str = "https://view.awsapps.com/start";
const KIRO_USER_AGENT: &str = "KiroIDE";

/// SSO OIDC Client for AWS authentication
pub struct SsoOidcClient {
    proxy: Option<ProxyConfig>,
    tls_backend: TlsBackend,
}

impl SsoOidcClient {
    pub fn new(proxy: Option<ProxyConfig>, tls_backend: TlsBackend) -> Self {
        Self { proxy, tls_backend }
    }

    fn get_oidc_endpoint(region: &str) -> String {
        format!("https://oidc.{}.amazonaws.com", region)
    }

    /// Register a new OIDC client with AWS
    pub async fn register_client(&self, region: &str) -> Result<RegisterClientResponse> {
        let endpoint = Self::get_oidc_endpoint(region);
        let url = format!("{}/client/register", endpoint);

        let payload = json!({
            "clientName": "Kiro IDE",
            "clientType": "public",
            "scopes": [
                "codewhisperer:completions",
                "codewhisperer:analysis",
                "codewhisperer:conversations",
                "codewhisperer:transformations",
                "codewhisperer:taskassist"
            ],
            "grantTypes": ["urn:ietf:params:oauth:grant-type:device_code", "refresh_token"]
        });

        let client = build_client(self.proxy.as_ref(), 30, self.tls_backend)?;
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("User-Agent", KIRO_USER_AGENT)
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Failed to register client (status {}): {}", status, body);
        }

        let result: RegisterClientResponse = response.json().await?;
        Ok(result)
    }

    /// Start device authorization flow
    pub async fn start_device_authorization(
        &self,
        client_id: &str,
        client_secret: &str,
        start_url: &str,
        region: &str,
    ) -> Result<StartDeviceAuthResponse> {
        let endpoint = Self::get_oidc_endpoint(region);
        let url = format!("{}/device_authorization", endpoint);

        let payload = json!({
            "clientId": client_id,
            "clientSecret": client_secret,
            "startUrl": start_url
        });

        let client = build_client(self.proxy.as_ref(), 30, self.tls_backend)?;
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("User-Agent", KIRO_USER_AGENT)
            .json(&payload)
            .send()
            .await?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!(
                "Failed to start device authorization (status {}): {}",
                status,
                body
            );
        }

        let result: StartDeviceAuthResponse = response.json().await?;
        Ok(result)
    }

    /// Poll for token after user authorization
    pub async fn create_token(
        &self,
        client_id: &str,
        client_secret: &str,
        device_code: &str,
        region: &str,
    ) -> Result<CreateTokenResult> {
        let endpoint = Self::get_oidc_endpoint(region);
        let url = format!("{}/token", endpoint);

        let payload = json!({
            "clientId": client_id,
            "clientSecret": client_secret,
            "deviceCode": device_code,
            "grantType": "urn:ietf:params:oauth:grant-type:device_code"
        });

        let client = build_client(self.proxy.as_ref(), 30, self.tls_backend)?;
        let response = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("User-Agent", KIRO_USER_AGENT)
            .json(&payload)
            .send()
            .await?;

        let status = response.status();

        if status.as_u16() == 400 {
            let body = response.text().await.unwrap_or_default();
            if let Ok(err_resp) = serde_json::from_str::<OidcErrorResponse>(&body) {
                return match err_resp.error.as_str() {
                    "authorization_pending" => Ok(CreateTokenResult::Pending),
                    "slow_down" => Ok(CreateTokenResult::SlowDown),
                    "expired_token" => Ok(CreateTokenResult::Expired),
                    _ => bail!("Token creation failed: {}", err_resp.error),
                };
            }
            bail!("Token creation failed: {}", body);
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Failed to create token (status {}): {}", status, body);
        }

        let result: CreateTokenResponse = response.json().await?;
        Ok(CreateTokenResult::Success(result))
    }

    /// Fetch profile ARN from CodeWhisperer API
    pub async fn fetch_profile_arn(&self, access_token: &str, region: &str) -> Option<String> {
        let host = format!("codewhisperer.{}.amazonaws.com", region);
        let url = format!("https://{}", host);

        let payload = json!({
            "origin": "AI_EDITOR"
        });

        let client = match build_client(self.proxy.as_ref(), 30, self.tls_backend) {
            Ok(c) => c,
            Err(_) => return None,
        };

        let response = client
            .post(&url)
            .header("Content-Type", "application/x-amz-json-1.0")
            .header("x-amz-target", "AmazonCodeWhispererService.ListProfiles")
            .header("Authorization", format!("Bearer {}", access_token))
            .header("Accept", "application/json")
            .json(&payload)
            .send()
            .await
            .ok()?;

        if !response.status().is_success() {
            return None;
        }

        let body: serde_json::Value = response.json().await.ok()?;

        // Try profileArn first, then profiles array
        if let Some(arn) = body.get("profileArn").and_then(|v| v.as_str()) {
            return Some(arn.to_string());
        }

        if let Some(profiles) = body.get("profiles").and_then(|v| v.as_array()) {
            if let Some(first) = profiles.first() {
                if let Some(arn) = first.get("arn").and_then(|v| v.as_str()) {
                    return Some(arn.to_string());
                }
            }
        }

        None
    }

    /// Get Builder ID start URL
    pub fn builder_id_start_url() -> &'static str {
        BUILDER_ID_START_URL
    }

    /// Get default region
    pub fn default_region() -> &'static str {
        DEFAULT_REGION
    }
}

/// Result of create_token operation
#[derive(Debug)]
pub enum CreateTokenResult {
    Success(CreateTokenResponse),
    Pending,
    SlowDown,
    Expired,
}
