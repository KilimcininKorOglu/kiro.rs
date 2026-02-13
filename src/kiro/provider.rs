//! Kiro API Provider
//!
//! Core component responsible for communicating with the Kiro API
//! Supports streaming and non-streaming requests
//! Supports multi-credential failover and retry

use reqwest::Client;
use reqwest::header::{AUTHORIZATION, CONNECTION, CONTENT_TYPE, HOST, HeaderMap, HeaderValue};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

use crate::http_client::{ProxyConfig, build_client};
use crate::kiro::errors::enhance_kiro_error;
use crate::kiro::machine_id;
use crate::kiro::model::credentials::KiroCredentials;
use crate::kiro::token_manager::{CallContext, MultiTokenManager};

/// Maximum retries per credential
const MAX_RETRIES_PER_CREDENTIAL: usize = 3;

/// Hard limit on total retries (to prevent infinite retries)
const MAX_TOTAL_RETRIES: usize = 9;

/// Enhance error message from Kiro API response body
///
/// Parses the response body as JSON and enhances the error message
/// with user-friendly text. Falls back to original body if parsing fails.
fn enhance_error_message(body: &str) -> String {
    if let Ok(error_json) = serde_json::from_str::<serde_json::Value>(body) {
        let error_info = enhance_kiro_error(&error_json);
        tracing::debug!(
            original_message = %error_info.original_message,
            reason = %error_info.reason,
            "Kiro API error enhanced"
        );
        error_info.user_message
    } else {
        body.to_string()
    }
}

/// Extract model name from Kiro API request body
///
/// Looks for model ID in conversationState.currentMessage.userInputMessage.modelId
fn extract_model_from_request(request_body: &str) -> Option<String> {
    let json: serde_json::Value = serde_json::from_str(request_body).ok()?;
    json.get("conversationState")?
        .get("currentMessage")?
        .get("userInputMessage")?
        .get("modelId")?
        .as_str()
        .map(|s| s.to_string())
}

/// Kiro API Provider
///
/// Core component responsible for communicating with the Kiro API
/// Supports multi-credential failover and retry mechanism
pub struct KiroProvider {
    token_manager: Arc<MultiTokenManager>,
    client: Client,
}

impl KiroProvider {
    /// Create a new KiroProvider instance
    pub fn new(token_manager: Arc<MultiTokenManager>) -> Self {
        Self::with_proxy(token_manager, None)
    }

    /// Create a KiroProvider instance with proxy configuration
    pub fn with_proxy(token_manager: Arc<MultiTokenManager>, proxy: Option<ProxyConfig>) -> Self {
        let client = build_client(proxy.as_ref(), 720, token_manager.config().tls_backend)
            .expect("Failed to create HTTP client");

        Self {
            token_manager,
            client,
        }
    }

    /// Get a reference to the token_manager
    pub fn token_manager(&self) -> &MultiTokenManager {
        &self.token_manager
    }

    /// Get API base URL (using config-level api_region)
    pub fn base_url(&self) -> String {
        format!(
            "https://q.{}.amazonaws.com/generateAssistantResponse",
            self.token_manager.config().effective_api_region()
        )
    }

    /// Get MCP API URL (using config-level api_region)
    pub fn mcp_url(&self) -> String {
        format!(
            "https://q.{}.amazonaws.com/mcp",
            self.token_manager.config().effective_api_region()
        )
    }

    /// Get API base domain (using config-level api_region)
    pub fn base_domain(&self) -> String {
        format!("q.{}.amazonaws.com", self.token_manager.config().effective_api_region())
    }

    /// Get credential-level API base URL
    fn base_url_for(&self, credentials: &KiroCredentials) -> String {
        format!(
            "https://q.{}.amazonaws.com/generateAssistantResponse",
            credentials.effective_api_region(self.token_manager.config())
        )
    }

    /// Get credential-level MCP API URL
    fn mcp_url_for(&self, credentials: &KiroCredentials) -> String {
        format!(
            "https://q.{}.amazonaws.com/mcp",
            credentials.effective_api_region(self.token_manager.config())
        )
    }

    /// Get credential-level API base domain
    fn base_domain_for(&self, credentials: &KiroCredentials) -> String {
        format!(
            "q.{}.amazonaws.com",
            credentials.effective_api_region(self.token_manager.config())
        )
    }

    /// Build request headers
    ///
    /// # Arguments
    /// * `ctx` - API call context containing credentials and token
    fn build_headers(&self, ctx: &CallContext) -> anyhow::Result<HeaderMap> {
        let config = self.token_manager.config();

        let machine_id = machine_id::generate_from_credentials(&ctx.credentials, config)
            .ok_or_else(|| anyhow::anyhow!("Failed to generate machine_id, please check credential configuration"))?;

        let kiro_version = &config.kiro_version;
        let os_name = &config.system_version;
        let node_version = &config.node_version;

        let x_amz_user_agent = format!("aws-sdk-js/1.0.27 KiroIDE-{}-{}", kiro_version, machine_id);

        let user_agent = format!(
            "aws-sdk-js/1.0.27 ua/2.1 os/{} lang/js md/nodejs#{} api/codewhispererstreaming#1.0.27 m/E KiroIDE-{}-{}",
            os_name, node_version, kiro_version, machine_id
        );

        let mut headers = HeaderMap::new();

        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "x-amzn-codewhisperer-optout",
            HeaderValue::from_static("true"),
        );
        headers.insert("x-amzn-kiro-agent-mode", HeaderValue::from_static("vibe"));
        headers.insert(
            "x-amz-user-agent",
            HeaderValue::from_str(&x_amz_user_agent).unwrap(),
        );
        headers.insert(
            reqwest::header::USER_AGENT,
            HeaderValue::from_str(&user_agent).unwrap(),
        );
        headers.insert(HOST, HeaderValue::from_str(&self.base_domain_for(&ctx.credentials)).unwrap());
        headers.insert(
            "amz-sdk-invocation-id",
            HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap(),
        );
        headers.insert(
            "amz-sdk-request",
            HeaderValue::from_static("attempt=1; max=3"),
        );
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {}", ctx.token)).unwrap(),
        );
        headers.insert(CONNECTION, HeaderValue::from_static("close"));

        Ok(headers)
    }

    /// Build MCP request headers
    fn build_mcp_headers(&self, ctx: &CallContext) -> anyhow::Result<HeaderMap> {
        let config = self.token_manager.config();

        let machine_id = machine_id::generate_from_credentials(&ctx.credentials, config)
            .ok_or_else(|| anyhow::anyhow!("Failed to generate machine_id, please check credential configuration"))?;

        let kiro_version = &config.kiro_version;
        let os_name = &config.system_version;
        let node_version = &config.node_version;

        let x_amz_user_agent = format!("aws-sdk-js/1.0.27 KiroIDE-{}-{}", kiro_version, machine_id);

        let user_agent = format!(
            "aws-sdk-js/1.0.27 ua/2.1 os/{} lang/js md/nodejs#{} api/codewhispererstreaming#1.0.27 m/E KiroIDE-{}-{}",
            os_name, node_version, kiro_version, machine_id
        );

        let mut headers = HeaderMap::new();

        // Add headers in strict order
        headers.insert("content-type", HeaderValue::from_static("application/json"));
        headers.insert(
            "x-amz-user-agent",
            HeaderValue::from_str(&x_amz_user_agent).unwrap(),
        );
        headers.insert("user-agent", HeaderValue::from_str(&user_agent).unwrap());
        headers.insert("host", HeaderValue::from_str(&self.base_domain_for(&ctx.credentials)).unwrap());
        headers.insert(
            "amz-sdk-invocation-id",
            HeaderValue::from_str(&Uuid::new_v4().to_string()).unwrap(),
        );
        headers.insert(
            "amz-sdk-request",
            HeaderValue::from_static("attempt=1; max=3"),
        );
        headers.insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {}", ctx.token)).unwrap(),
        );
        headers.insert("Connection", HeaderValue::from_static("close"));

        Ok(headers)
    }

    /// Send non-streaming API request
    ///
    /// Supports multi-credential failover:
    /// - 400 Bad Request: Return error directly, does not count as credential failure
    /// - 401/403: Treated as credential/permission issue, counts as failure and allows failover
    /// - 402 MONTHLY_REQUEST_COUNT: Treated as quota exhausted, disables credential and switches
    /// - 429/5xx/network transient errors: Retry but don't disable or switch credentials (to avoid locking all credentials)
    ///
    /// # Arguments
    /// * `request_body` - JSON formatted request body string
    ///
    /// # Returns
    /// Returns raw HTTP Response without parsing
    pub async fn call_api(&self, request_body: &str) -> anyhow::Result<reqwest::Response> {
        self.call_api_with_retry(request_body, false).await
    }

    /// Send streaming API request
    ///
    /// Supports multi-credential failover:
    /// - 400 Bad Request: Return error directly, does not count as credential failure
    /// - 401/403: Treated as credential/permission issue, counts as failure and allows failover
    /// - 402 MONTHLY_REQUEST_COUNT: Treated as quota exhausted, disables credential and switches
    /// - 429/5xx/network transient errors: Retry but don't disable or switch credentials (to avoid locking all credentials)
    ///
    /// # Arguments
    /// * `request_body` - JSON formatted request body string
    ///
    /// # Returns
    /// Returns raw HTTP Response, caller is responsible for handling streaming data
    pub async fn call_api_stream(&self, request_body: &str) -> anyhow::Result<reqwest::Response> {
        self.call_api_with_retry(request_body, true).await
    }

    /// Send MCP API request
    ///
    /// Used for tool calls like WebSearch
    ///
    /// # Arguments
    /// * `request_body` - JSON formatted MCP request body string
    ///
    /// # Returns
    /// Returns raw HTTP Response
    pub async fn call_mcp(&self, request_body: &str) -> anyhow::Result<reqwest::Response> {
        self.call_mcp_with_retry(request_body).await
    }

    /// Internal method: MCP API call with retry logic
    async fn call_mcp_with_retry(&self, request_body: &str) -> anyhow::Result<reqwest::Response> {
        let total_credentials = self.token_manager.total_count();
        let max_retries = (total_credentials * MAX_RETRIES_PER_CREDENTIAL).min(MAX_TOTAL_RETRIES);
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..max_retries {
            // Get call context (MCP doesn't need model filtering)
            let ctx = match self.token_manager.acquire_context(None).await {
                Ok(c) => c,
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            };

            let url = self.mcp_url_for(&ctx.credentials);
            let headers = match self.build_mcp_headers(&ctx) {
                Ok(h) => h,
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            };

            // Send request
            let response = match self
                .client
                .post(&url)
                .headers(headers)
                .body(request_body.to_string())
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::warn!(
                        "MCP request failed to send (attempt {}/{}): {}",
                        attempt + 1,
                        max_retries,
                        e
                    );
                    last_error = Some(e.into());
                    if attempt + 1 < max_retries {
                        sleep(Self::retry_delay(attempt)).await;
                    }
                    continue;
                }
            };

            let status = response.status();

            // Successful response
            if status.is_success() {
                let credential_info = ctx.credentials.email.as_deref().unwrap_or("unknown");
                tracing::info!(
                    credential_id = ctx.id,
                    credential_email = credential_info,
                    "MCP request succeeded"
                );
                self.token_manager.report_success(ctx.id);
                return Ok(response);
            }

            // Failed response
            let body = response.text().await.unwrap_or_default();

            // 402 quota exhausted
            if status.as_u16() == 402 && Self::is_monthly_request_limit(&body) {
                let has_available = self.token_manager.report_quota_exhausted(ctx.id);
                if !has_available {
                    anyhow::bail!("MCP request failed (all credentials exhausted): {} {}", status, body);
                }
                last_error = Some(anyhow::anyhow!("MCP request failed: {} {}", status, body));
                continue;
            }

            // 400 Bad Request
            if status.as_u16() == 400 {
                let enhanced_msg = enhance_error_message(&body);
                anyhow::bail!("MCP request failed: {} - {}", status, enhanced_msg);
            }

            // 401/403 credential issue
            if matches!(status.as_u16(), 401 | 403) {
                let has_available = self.token_manager.report_failure(ctx.id);
                if !has_available {
                    anyhow::bail!("MCP request failed (all credentials exhausted): {} {}", status, body);
                }
                last_error = Some(anyhow::anyhow!("MCP request failed: {} {}", status, body));
                continue;
            }

            // Transient error
            if matches!(status.as_u16(), 408 | 429) || status.is_server_error() {
                tracing::warn!(
                    "MCP request failed (upstream transient error, attempt {}/{}): {} {}",
                    attempt + 1,
                    max_retries,
                    status,
                    body
                );
                last_error = Some(anyhow::anyhow!("MCP request failed: {} {}", status, body));
                if attempt + 1 < max_retries {
                    sleep(Self::retry_delay(attempt)).await;
                }
                continue;
            }

            // Other 4xx
            if status.is_client_error() {
                let enhanced_msg = enhance_error_message(&body);
                anyhow::bail!("MCP request failed: {} - {}", status, enhanced_msg);
            }

            // Fallback
            last_error = Some(anyhow::anyhow!("MCP request failed: {} {}", status, body));
            if attempt + 1 < max_retries {
                sleep(Self::retry_delay(attempt)).await;
            }
        }

        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!("MCP request failed: reached maximum retry count ({} times)", max_retries)
        }))
    }

    /// Internal method: API call with retry logic
    ///
    /// Retry strategy:
    /// - Each credential retries up to MAX_RETRIES_PER_CREDENTIAL times
    /// - Total retries = min(credential count × retries per credential, MAX_TOTAL_RETRIES)
    /// - Hard limit of 9 times to prevent infinite retries
    async fn call_api_with_retry(
        &self,
        request_body: &str,
        is_stream: bool,
    ) -> anyhow::Result<reqwest::Response> {
        let total_credentials = self.token_manager.total_count();
        let max_retries = (total_credentials * MAX_RETRIES_PER_CREDENTIAL).min(MAX_TOTAL_RETRIES);
        let mut last_error: Option<anyhow::Error> = None;
        let api_type = if is_stream { "streaming" } else { "non-streaming" };

        // Extract model from request for credential filtering
        let model = extract_model_from_request(request_body);

        for attempt in 0..max_retries {
            // Get call context (binds index, credentials, token)
            let ctx = match self.token_manager.acquire_context(model.as_deref()).await {
                Ok(c) => c,
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            };

            let url = self.base_url_for(&ctx.credentials);
            let headers = match self.build_headers(&ctx) {
                Ok(h) => h,
                Err(e) => {
                    last_error = Some(e);
                    continue;
                }
            };

            // Send request
            let response = match self
                .client
                .post(&url)
                .headers(headers)
                .body(request_body.to_string())
                .send()
                .await
            {
                Ok(resp) => resp,
                Err(e) => {
                    tracing::warn!(
                        "API request failed to send (attempt {}/{}): {}",
                        attempt + 1,
                        max_retries,
                        e
                    );
                    // Network errors are usually upstream/link transient issues, should not cause "disable credential" or "switch credential"
                    // (Otherwise network jitter would mistakenly disable all credentials, requiring restart to recover)
                    last_error = Some(e.into());
                    if attempt + 1 < max_retries {
                        sleep(Self::retry_delay(attempt)).await;
                    }
                    continue;
                }
            };

            let status = response.status();

            // Successful response
            if status.is_success() {
                let credential_info = ctx.credentials.email.as_deref().unwrap_or("unknown");
                tracing::info!(
                    credential_id = ctx.id,
                    credential_email = credential_info,
                    "API request succeeded"
                );
                self.token_manager.report_success(ctx.id);
                return Ok(response);
            }

            // Failed response: read body for logging/error messages
            let body = response.text().await.unwrap_or_default();

            // 402 Payment Required with quota exhausted: disable credential and failover
            if status.as_u16() == 402 && Self::is_monthly_request_limit(&body) {
                tracing::warn!(
                    "API request failed (quota exhausted, disabling credential and switching, attempt {}/{}): {} {}",
                    attempt + 1,
                    max_retries,
                    status,
                    body
                );

                let has_available = self.token_manager.report_quota_exhausted(ctx.id);
                if !has_available {
                    anyhow::bail!(
                        "{} API request failed (all credentials exhausted): {} {}",
                        api_type,
                        status,
                        body
                    );
                }

                last_error = Some(anyhow::anyhow!(
                    "{} API request failed: {} {}",
                    api_type,
                    status,
                    body
                ));
                continue;
            }

            // 400 Bad Request - request issue, retry/switch credential is meaningless
            if status.as_u16() == 400 {
                let enhanced_msg = enhance_error_message(&body);
                anyhow::bail!("{} API request failed: {} - {}", api_type, status, enhanced_msg);
            }

            // 401/403 - more likely credential/permission issue: count as failure and allow failover
            if matches!(status.as_u16(), 401 | 403) {
                tracing::warn!(
                    "API request failed (possibly credential error, attempt {}/{}): {} {}",
                    attempt + 1,
                    max_retries,
                    status,
                    body
                );

                let has_available = self.token_manager.report_failure(ctx.id);
                if !has_available {
                    anyhow::bail!(
                        "{} API request failed (all credentials exhausted): {} {}",
                        api_type,
                        status,
                        body
                    );
                }

                last_error = Some(anyhow::anyhow!(
                    "{} API request failed: {} {}",
                    api_type,
                    status,
                    body
                ));
                continue;
            }

            // 429/408/5xx - transient upstream error: retry but don't disable or switch credentials
            // (To avoid 429 high traffic / 502 high load transient errors locking all credentials)
            if matches!(status.as_u16(), 408 | 429) || status.is_server_error() {
                tracing::warn!(
                    "API request failed (upstream transient error, attempt {}/{}): {} {}",
                    attempt + 1,
                    max_retries,
                    status,
                    body
                );
                last_error = Some(anyhow::anyhow!(
                    "{} API request failed: {} {}",
                    api_type,
                    status,
                    body
                ));
                if attempt + 1 < max_retries {
                    sleep(Self::retry_delay(attempt)).await;
                }
                continue;
            }

            // Other 4xx - usually request/configuration issue: return directly, don't count as credential failure
            if status.is_client_error() {
                let enhanced_msg = enhance_error_message(&body);
                anyhow::bail!("{} API request failed: {} - {}", api_type, status, enhanced_msg);
            }

            // Fallback: treat as retryable transient error (don't switch credentials)
            tracing::warn!(
                "API request failed (unknown error, attempt {}/{}): {} {}",
                attempt + 1,
                max_retries,
                status,
                body
            );
            last_error = Some(anyhow::anyhow!(
                "{} API request failed: {} {}",
                api_type,
                status,
                body
            ));
            if attempt + 1 < max_retries {
                sleep(Self::retry_delay(attempt)).await;
            }
        }

        // All retries failed
        Err(last_error.unwrap_or_else(|| {
            anyhow::anyhow!(
                "{} API request failed: reached maximum retry count ({} times)",
                api_type,
                max_retries
            )
        }))
    }

    fn retry_delay(attempt: usize) -> Duration {
        // Exponential backoff + small jitter to avoid amplifying failures during upstream jitter
        const BASE_MS: u64 = 200;
        const MAX_MS: u64 = 2_000;
        let exp = BASE_MS.saturating_mul(2u64.saturating_pow(attempt.min(6) as u32));
        let backoff = exp.min(MAX_MS);
        let jitter_max = (backoff / 4).max(1);
        let jitter = fastrand::u64(0..=jitter_max);
        Duration::from_millis(backoff.saturating_add(jitter))
    }

    fn is_monthly_request_limit(body: &str) -> bool {
        if body.contains("MONTHLY_REQUEST_COUNT") {
            return true;
        }

        let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
            return false;
        };

        if value
            .get("reason")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v == "MONTHLY_REQUEST_COUNT")
        {
            return true;
        }

        value
            .pointer("/error/reason")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v == "MONTHLY_REQUEST_COUNT")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kiro::token_manager::CallContext;
    use crate::model::config::Config;

    fn create_test_provider(config: Config, credentials: KiroCredentials) -> KiroProvider {
        let tm = MultiTokenManager::new(config, vec![credentials], None, None, false).unwrap();
        KiroProvider::new(Arc::new(tm))
    }

    #[test]
    fn test_base_url() {
        let config = Config::default();
        let credentials = KiroCredentials::default();
        let provider = create_test_provider(config, credentials);
        assert!(provider.base_url().contains("amazonaws.com"));
        assert!(provider.base_url().contains("generateAssistantResponse"));
    }

    #[test]
    fn test_base_domain() {
        let mut config = Config::default();
        config.region = "us-east-1".to_string();
        let credentials = KiroCredentials::default();
        let provider = create_test_provider(config, credentials);
        assert_eq!(provider.base_domain(), "q.us-east-1.amazonaws.com");
    }

    #[test]
    fn test_build_headers() {
        let mut config = Config::default();
        config.region = "us-east-1".to_string();
        config.kiro_version = "0.8.0".to_string();

        let mut credentials = KiroCredentials::default();
        credentials.profile_arn = Some("arn:aws:sso::123456789:profile/test".to_string());
        credentials.refresh_token = Some("a".repeat(150));

        let provider = create_test_provider(config, credentials.clone());
        let ctx = CallContext {
            id: 1,
            credentials,
            token: "test_token".to_string(),
        };
        let headers = provider.build_headers(&ctx).unwrap();

        assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
        assert_eq!(headers.get("x-amzn-codewhisperer-optout").unwrap(), "true");
        assert_eq!(headers.get("x-amzn-kiro-agent-mode").unwrap(), "vibe");
        assert!(
            headers
                .get(AUTHORIZATION)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("Bearer ")
        );
        assert_eq!(headers.get(CONNECTION).unwrap(), "close");
    }

    #[test]
    fn test_is_monthly_request_limit_detects_reason() {
        let body = r#"{"message":"You have reached the limit.","reason":"MONTHLY_REQUEST_COUNT"}"#;
        assert!(KiroProvider::is_monthly_request_limit(body));
    }

    #[test]
    fn test_is_monthly_request_limit_nested_reason() {
        let body = r#"{"error":{"reason":"MONTHLY_REQUEST_COUNT"}}"#;
        assert!(KiroProvider::is_monthly_request_limit(body));
    }

    #[test]
    fn test_is_monthly_request_limit_false() {
        let body = r#"{"message":"nope","reason":"DAILY_REQUEST_COUNT"}"#;
        assert!(!KiroProvider::is_monthly_request_limit(body));
    }
}
