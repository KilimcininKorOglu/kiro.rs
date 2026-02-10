//! Token management module
//!
//! Handles Token expiration detection and refresh, supports Social and IdC authentication methods
//! Supports single credential (TokenManager) and multi-credential (MultiTokenManager) management

use anyhow::bail;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as TokioMutex;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration as StdDuration, Instant};

use crate::http_client::{ProxyConfig, build_client};
use crate::kiro::machine_id;
use crate::kiro::model::credentials::KiroCredentials;
use crate::kiro::model::token_refresh::{
    IdcRefreshRequest, IdcRefreshResponse, RefreshRequest, RefreshResponse,
};
use crate::kiro::model::usage_limits::UsageLimitsResponse;
use crate::model::config::Config;

/// JWT claims structure for extracting email
#[derive(Debug, Deserialize)]
struct JwtClaims {
    email: Option<String>,
    preferred_username: Option<String>,
    sub: Option<String>,
}

/// Extract email from JWT access token
///
/// JWT format: header.payload.signature
/// Payload is base64url-encoded JSON containing user claims
fn extract_email_from_jwt(access_token: &str) -> Option<String> {
    let parts: Vec<&str> = access_token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    // Decode payload (second part)
    let payload = parts[1];
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: JwtClaims = serde_json::from_slice(&decoded).ok()?;

    // Priority: email > preferred_username (if contains @) > sub (if contains @)
    if let Some(email) = claims.email {
        if !email.is_empty() {
            return Some(email);
        }
    }

    if let Some(username) = claims.preferred_username {
        if username.contains('@') {
            return Some(username);
        }
    }

    if let Some(sub) = claims.sub {
        if sub.contains('@') {
            return Some(sub);
        }
    }

    None
}

/// Token manager
///
/// Manages credentials and automatic Token refresh
pub struct TokenManager {
    config: Config,
    credentials: KiroCredentials,
    proxy: Option<ProxyConfig>,
}

impl TokenManager {
    /// Create new TokenManager instance
    pub fn new(config: Config, credentials: KiroCredentials, proxy: Option<ProxyConfig>) -> Self {
        Self {
            config,
            credentials,
            proxy,
        }
    }

    /// Get credentials reference
    pub fn credentials(&self) -> &KiroCredentials {
        &self.credentials
    }

    /// Get config reference
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Ensure valid access Token
    ///
    /// Automatically refreshes if Token is expired or about to expire
    pub async fn ensure_valid_token(&mut self) -> anyhow::Result<String> {
        if is_token_expired(&self.credentials) || is_token_expiring_soon(&self.credentials) {
            self.credentials =
                refresh_token(&self.credentials, &self.config, self.proxy.as_ref()).await?;

            // Check token validity again after refresh
            if is_token_expired(&self.credentials) {
                anyhow::bail!("Refreshed Token is still invalid or expired");
            }
        }

        self.credentials
            .access_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No available accessToken"))
    }

    /// Get usage limits information
    ///
    /// Calls getUsageLimits API to query current account usage limits
    pub async fn get_usage_limits(&mut self) -> anyhow::Result<UsageLimitsResponse> {
        let token = self.ensure_valid_token().await?;
        get_usage_limits(&self.credentials, &self.config, &token, self.proxy.as_ref()).await
    }
}

/// Check if Token expires within specified time
pub(crate) fn is_token_expiring_within(
    credentials: &KiroCredentials,
    minutes: i64,
) -> Option<bool> {
    credentials
        .expires_at
        .as_ref()
        .and_then(|expires_at| DateTime::parse_from_rfc3339(expires_at).ok())
        .map(|expires| expires <= Utc::now() + Duration::minutes(minutes))
}

/// Check if Token is expired (with 5 minute buffer)
pub(crate) fn is_token_expired(credentials: &KiroCredentials) -> bool {
    is_token_expiring_within(credentials, 5).unwrap_or(true)
}

/// Check if Token is expiring soon (within 10 minutes)
pub(crate) fn is_token_expiring_soon(credentials: &KiroCredentials) -> bool {
    is_token_expiring_within(credentials, 10).unwrap_or(false)
}

fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    format!("{:x}", result)
}

/// Validate basic validity of refreshToken
pub(crate) fn validate_refresh_token(credentials: &KiroCredentials) -> anyhow::Result<()> {
    let refresh_token = credentials
        .refresh_token
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing refreshToken"))?;

    if refresh_token.is_empty() {
        bail!("refreshToken is empty");
    }

    if refresh_token.len() < 100 || refresh_token.ends_with("...") || refresh_token.contains("...")
    {
        bail!(
            "refreshToken has been truncated (length: {} characters).\n\
             This is usually intentionally truncated by Kiro IDE to prevent credentials from being used by third-party tools.",
            refresh_token.len()
        );
    }

    Ok(())
}

/// Refresh Token
pub(crate) async fn refresh_token(
    credentials: &KiroCredentials,
    config: &Config,
    proxy: Option<&ProxyConfig>,
) -> anyhow::Result<KiroCredentials> {
    validate_refresh_token(credentials)?;

    // Select refresh method based on auth_method
    // If auth_method is not specified, auto-detect based on presence of clientId/clientSecret
    let auth_method = credentials.auth_method.as_deref().unwrap_or_else(|| {
        if credentials.client_id.is_some() && credentials.client_secret.is_some() {
            "idc"
        } else {
            "social"
        }
    });

    if auth_method.eq_ignore_ascii_case("idc")
        || auth_method.eq_ignore_ascii_case("builder-id")
        || auth_method.eq_ignore_ascii_case("iam")
    {
        refresh_idc_token(credentials, config, proxy).await
    } else {
        refresh_social_token(credentials, config, proxy).await
    }
}

/// Refresh Social Token
async fn refresh_social_token(
    credentials: &KiroCredentials,
    config: &Config,
    proxy: Option<&ProxyConfig>,
) -> anyhow::Result<KiroCredentials> {
    tracing::info!("Refreshing Social Token...");

    let refresh_token = credentials.refresh_token.as_ref().unwrap();
    // Priority: credential.auth_region > credential.region > config.auth_region > config.region
    let region = credentials.effective_auth_region(config);

    let refresh_url = format!("https://prod.{}.auth.desktop.kiro.dev/refreshToken", region);
    let refresh_domain = format!("prod.{}.auth.desktop.kiro.dev", region);
    let machine_id = machine_id::generate_from_credentials(credentials, config)
        .ok_or_else(|| anyhow::anyhow!("Unable to generate machineId"))?;
    let kiro_version = &config.kiro_version;

    let client = build_client(proxy, 60, config.tls_backend)?;
    let body = RefreshRequest {
        refresh_token: refresh_token.to_string(),
    };

    let response = client
        .post(&refresh_url)
        .header("Accept", "application/json, text/plain, */*")
        .header("Content-Type", "application/json")
        .header(
            "User-Agent",
            format!("KiroIDE-{}-{}", kiro_version, machine_id),
        )
        .header("Accept-Encoding", "gzip, compress, deflate, br")
        .header("host", &refresh_domain)
        .header("Connection", "close")
        .json(&body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let error_msg = match status.as_u16() {
            401 => "OAuth credentials expired or invalid, re-authentication required",
            403 => "Insufficient permissions, unable to refresh Token",
            429 => "Too many requests, rate limited",
            500..=599 => "Server error, AWS OAuth service temporarily unavailable",
            _ => "Token refresh failed",
        };
        bail!("{}: {} {}", error_msg, status, body_text);
    }

    let data: RefreshResponse = response.json().await?;

    let mut new_credentials = credentials.clone();
    new_credentials.access_token = Some(data.access_token.clone());

    // Extract email from JWT access token if not already set
    if new_credentials.email.is_none() {
        if let Some(email) = extract_email_from_jwt(&data.access_token) {
            tracing::info!("Extracted email from JWT: {}", email);
            new_credentials.email = Some(email);
        }
    }

    if let Some(new_refresh_token) = data.refresh_token {
        new_credentials.refresh_token = Some(new_refresh_token);
    }

    if let Some(profile_arn) = data.profile_arn {
        new_credentials.profile_arn = Some(profile_arn);
    }

    if let Some(expires_in) = data.expires_in {
        let expires_at = Utc::now() + Duration::seconds(expires_in);
        new_credentials.expires_at = Some(expires_at.to_rfc3339());
    }

    Ok(new_credentials)
}

/// x-amz-user-agent header required for IdC Token refresh
const IDC_AMZ_USER_AGENT: &str = "aws-sdk-js/3.738.0 ua/2.1 os/other lang/js md/browser#unknown_unknown api/sso-oidc#3.738.0 m/E KiroIDE";

/// Refresh IdC Token (AWS SSO OIDC)
async fn refresh_idc_token(
    credentials: &KiroCredentials,
    config: &Config,
    proxy: Option<&ProxyConfig>,
) -> anyhow::Result<KiroCredentials> {
    tracing::info!("Refreshing IdC Token...");

    let refresh_token = credentials.refresh_token.as_ref().unwrap();
    let client_id = credentials
        .client_id
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("IdC refresh requires clientId"))?;
    let client_secret = credentials
        .client_secret
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("IdC refresh requires clientSecret"))?;

    // Priority: credential.auth_region > credential.region > config.auth_region > config.region
    let region = credentials.effective_auth_region(config);
    let refresh_url = format!("https://oidc.{}.amazonaws.com/token", region);

    let client = build_client(proxy, 60, config.tls_backend)?;
    let body = IdcRefreshRequest {
        client_id: client_id.to_string(),
        client_secret: client_secret.to_string(),
        refresh_token: refresh_token.to_string(),
        grant_type: "refresh_token".to_string(),
    };

    let response = client
        .post(&refresh_url)
        .header("Content-Type", "application/json")
        .header("Host", format!("oidc.{}.amazonaws.com", region))
        .header("Connection", "keep-alive")
        .header("x-amz-user-agent", IDC_AMZ_USER_AGENT)
        .header("Accept", "*/*")
        .header("Accept-Language", "*")
        .header("sec-fetch-mode", "cors")
        .header("User-Agent", "node")
        .header("Accept-Encoding", "br, gzip, deflate")
        .json(&body)
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let error_msg = match status.as_u16() {
            401 => "IdC credentials expired or invalid, re-authentication required",
            403 => "Insufficient permissions, unable to refresh Token",
            429 => "Too many requests, rate limited",
            500..=599 => "Server error, AWS OIDC service temporarily unavailable",
            _ => "IdC Token refresh failed",
        };
        bail!("{}: {} {}", error_msg, status, body_text);
    }

    let data: IdcRefreshResponse = response.json().await?;

    let mut new_credentials = credentials.clone();
    new_credentials.access_token = Some(data.access_token.clone());

    // Extract email from JWT access token if not already set
    if new_credentials.email.is_none() {
        if let Some(email) = extract_email_from_jwt(&data.access_token) {
            tracing::info!("Extracted email from JWT: {}", email);
            new_credentials.email = Some(email);
        }
    }

    if let Some(new_refresh_token) = data.refresh_token {
        new_credentials.refresh_token = Some(new_refresh_token);
    }

    if let Some(expires_in) = data.expires_in {
        let expires_at = Utc::now() + Duration::seconds(expires_in);
        new_credentials.expires_at = Some(expires_at.to_rfc3339());
    }

    Ok(new_credentials)
}

/// x-amz-user-agent header prefix required for getUsageLimits API
const USAGE_LIMITS_AMZ_USER_AGENT_PREFIX: &str = "aws-sdk-js/1.0.0";

/// Get usage limits information
pub(crate) async fn get_usage_limits(
    credentials: &KiroCredentials,
    config: &Config,
    token: &str,
    proxy: Option<&ProxyConfig>,
) -> anyhow::Result<UsageLimitsResponse> {
    tracing::debug!("Getting usage limits information...");

    // Priority: credential.api_region > config.api_region > config.region
    let region = credentials.effective_api_region(config);
    let host = format!("q.{}.amazonaws.com", region);
    let machine_id = machine_id::generate_from_credentials(credentials, config)
        .ok_or_else(|| anyhow::anyhow!("Unable to generate machineId"))?;
    let kiro_version = &config.kiro_version;

    // Build URL
    let mut url = format!(
        "https://{}/getUsageLimits?origin=AI_EDITOR&resourceType=AGENTIC_REQUEST",
        host
    );

    // profileArn is optional
    if let Some(profile_arn) = &credentials.profile_arn {
        url.push_str(&format!("&profileArn={}", urlencoding::encode(profile_arn)));
    }

    // Build User-Agent headers
    let user_agent = format!(
        "aws-sdk-js/1.0.0 ua/2.1 os/darwin#24.6.0 lang/js md/nodejs#22.21.1 \
         api/codewhispererruntime#1.0.0 m/N,E KiroIDE-{}-{}",
        kiro_version, machine_id
    );
    let amz_user_agent = format!(
        "{} KiroIDE-{}-{}",
        USAGE_LIMITS_AMZ_USER_AGENT_PREFIX, kiro_version, machine_id
    );

    let client = build_client(proxy, 60, config.tls_backend)?;

    let response = client
        .get(&url)
        .header("x-amz-user-agent", &amz_user_agent)
        .header("User-Agent", &user_agent)
        .header("host", &host)
        .header("amz-sdk-invocation-id", uuid::Uuid::new_v4().to_string())
        .header("amz-sdk-request", "attempt=1; max=1")
        .header("Authorization", format!("Bearer {}", token))
        .header("Connection", "close")
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let error_msg = match status.as_u16() {
            401 => "Authentication failed, Token invalid or expired",
            403 => "Insufficient permissions, unable to get usage limits",
            429 => "Too many requests, rate limited",
            500..=599 => "Server error, AWS service temporarily unavailable",
            _ => "Failed to get usage limits",
        };
        bail!("{}: {} {}", error_msg, status, body_text);
    }

    let data: UsageLimitsResponse = response.json().await?;
    Ok(data)
}

// ============================================================================
// Multi-credential Token Manager
// ============================================================================

/// Single credential entry state
struct CredentialEntry {
    /// Credential unique ID
    id: u64,
    /// Credential information
    credentials: KiroCredentials,
    /// Consecutive API call failure count
    failure_count: u32,
    /// Whether disabled
    disabled: bool,
    /// Disabled reason (to distinguish manual vs automatic disable, for self-healing)
    disabled_reason: Option<DisabledReason>,
    /// API call success count
    success_count: u64,
    /// Last API call time (RFC3339 format)
    last_used_at: Option<String>,
}

/// Disabled reason
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DisabledReason {
    /// Manually disabled via Admin API
    Manual,
    /// Automatically disabled after reaching failure threshold
    TooManyFailures,
    /// Quota exhausted (e.g., MONTHLY_REQUEST_COUNT)
    QuotaExceeded,
}

/// Statistics persistence entry
#[derive(Serialize, Deserialize)]
struct StatsEntry {
    success_count: u64,
    last_used_at: Option<String>,
}

// ============================================================================
// Admin API public structures
// ============================================================================

/// Credential entry snapshot (for Admin API read)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialEntrySnapshot {
    /// Credential unique ID
    pub id: u64,
    /// Priority
    pub priority: u32,
    /// Whether disabled
    pub disabled: bool,
    /// Consecutive failure count
    pub failure_count: u32,
    /// Authentication method
    pub auth_method: Option<String>,
    /// Whether has Profile ARN
    pub has_profile_arn: bool,
    /// Token expiration time
    pub expires_at: Option<String>,
    /// SHA-256 hash of refreshToken (for frontend duplicate detection)
    pub refresh_token_hash: Option<String>,
    /// User email (for frontend display)
    pub email: Option<String>,
    /// API call success count
    pub success_count: u64,
    /// Last API call time (RFC3339 format)
    pub last_used_at: Option<String>,
}

/// Credential manager state snapshot
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerSnapshot {
    /// Credential entry list
    pub entries: Vec<CredentialEntrySnapshot>,
    /// Current active credential ID
    pub current_id: u64,
    /// Total credential count
    pub total: usize,
    /// Available credential count
    pub available: usize,
}

/// Multi-credential Token manager
///
/// Supports management of multiple credentials, implements fixed priority + failover strategy
/// Failure statistics based on API call results, not Token refresh results
pub struct MultiTokenManager {
    config: Config,
    proxy: Option<ProxyConfig>,
    /// Credential entry list
    entries: Mutex<Vec<CredentialEntry>>,
    /// Current active credential ID
    current_id: Mutex<u64>,
    /// Token refresh lock, ensures only one refresh operation at a time
    refresh_lock: TokioMutex<()>,
    /// Credentials file path (for write-back)
    credentials_path: Option<PathBuf>,
    /// Whether multiple credentials format (only array format writes back, auto-upgrades to true when adding credentials)
    is_multiple_format: Mutex<bool>,
    /// Load balancing mode (modifiable at runtime)
    load_balancing_mode: Mutex<String>,
    /// Last statistics persistence time (for debounce)
    last_stats_save_at: Mutex<Option<Instant>>,
    /// Whether statistics data has unsaved updates
    stats_dirty: AtomicBool,
}

/// Maximum API call failures per credential
const MAX_FAILURES_PER_CREDENTIAL: u32 = 3;
/// Statistics persistence debounce interval
const STATS_SAVE_DEBOUNCE: StdDuration = StdDuration::from_secs(30);

/// API call context
///
/// Call context bound to specific credential, ensures consistency of token, credentials and id
/// Used to solve current_id race condition during concurrent calls
#[derive(Clone)]
pub struct CallContext {
    /// Credential ID (for report_success/report_failure)
    pub id: u64,
    /// Credential information (for building request headers)
    pub credentials: KiroCredentials,
    /// Access Token
    pub token: String,
}

impl MultiTokenManager {
    /// Create multi-credential Token manager
    ///
    /// # Arguments
    /// * `config` - Application configuration
    /// * `credentials` - Credentials list
    /// * `proxy` - Optional proxy configuration
    /// * `credentials_path` - Credentials file path (for write-back)
    /// * `is_multiple_format` - Whether multiple credentials format (only array format writes back)
    pub fn new(
        config: Config,
        credentials: Vec<KiroCredentials>,
        proxy: Option<ProxyConfig>,
        credentials_path: Option<PathBuf>,
        is_multiple_format: bool,
    ) -> anyhow::Result<Self> {
        // Calculate current max ID, assign new ID to credentials without ID
        let max_existing_id = credentials.iter().filter_map(|c| c.id).max().unwrap_or(0);
        let mut next_id = max_existing_id + 1;
        let mut has_new_ids = false;
        let mut has_new_machine_ids = false;
        let config_ref = &config;

        let entries: Vec<CredentialEntry> = credentials
            .into_iter()
            .map(|mut cred| {
                cred.canonicalize_auth_method();
                let id = cred.id.unwrap_or_else(|| {
                    let id = next_id;
                    next_id += 1;
                    cred.id = Some(id);
                    has_new_ids = true;
                    id
                });
                if cred.machine_id.is_none() {
                    if let Some(machine_id) =
                        machine_id::generate_from_credentials(&cred, config_ref)
                    {
                        cred.machine_id = Some(machine_id);
                        has_new_machine_ids = true;
                    }
                }
                CredentialEntry {
                    id,
                    credentials: cred,
                    failure_count: 0,
                    disabled: false,
                    disabled_reason: None,
                    success_count: 0,
                    last_used_at: None,
                }
            })
            .collect();

        // Detect duplicate IDs
        let mut seen_ids = std::collections::HashSet::new();
        let mut duplicate_ids = Vec::new();
        for entry in &entries {
            if !seen_ids.insert(entry.id) {
                duplicate_ids.push(entry.id);
            }
        }
        if !duplicate_ids.is_empty() {
            anyhow::bail!("Duplicate credential IDs detected: {:?}", duplicate_ids);
        }

        // Select initial credential: highest priority (lowest priority number), 0 if no credentials
        let initial_id = entries
            .iter()
            .min_by_key(|e| e.credentials.priority)
            .map(|e| e.id)
            .unwrap_or(0);

        let load_balancing_mode = config.load_balancing_mode.clone();
        let manager = Self {
            config,
            proxy,
            entries: Mutex::new(entries),
            current_id: Mutex::new(initial_id),
            refresh_lock: TokioMutex::new(()),
            credentials_path,
            is_multiple_format: Mutex::new(is_multiple_format),
            load_balancing_mode: Mutex::new(load_balancing_mode),
            last_stats_save_at: Mutex::new(None),
            stats_dirty: AtomicBool::new(false),
        };

        // If new IDs or machineIds were assigned, persist to config file immediately
        if has_new_ids || has_new_machine_ids {
            if let Err(e) = manager.persist_credentials() {
                tracing::warn!("Failed to persist after completing credential ID/machineId: {}", e);
            } else {
                tracing::info!("Completed credential ID/machineId and wrote back to config file");
            }
        }

        // Load persisted statistics (success_count, last_used_at)
        manager.load_stats();

        Ok(manager)
    }

    /// Get config reference
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Get clone of current active credential
    pub fn credentials(&self) -> KiroCredentials {
        let entries = self.entries.lock();
        let current_id = *self.current_id.lock();
        entries
            .iter()
            .find(|e| e.id == current_id)
            .map(|e| e.credentials.clone())
            .unwrap_or_default()
    }

    /// Get total credential count
    pub fn total_count(&self) -> usize {
        self.entries.lock().len()
    }

    /// Get available credential count
    pub fn available_count(&self) -> usize {
        self.entries.lock().iter().filter(|e| !e.disabled).count()
    }

    /// Select next credential based on load balancing mode
    ///
    /// - priority mode: Select highest priority (lowest priority number) available credential
    /// - balanced mode: Round-robin select available credentials
    /// - If model contains "opus", filter out FREE tier accounts in balanced mode
    fn select_next_credential(&self, model: Option<&str>) -> Option<(u64, KiroCredentials)> {
        let entries = self.entries.lock();
        let mode = self.load_balancing_mode.lock().clone();
        let mode = mode.as_str();

        // Check if requesting Opus model
        let is_opus = model
            .map(|m| m.to_lowercase().contains("opus"))
            .unwrap_or(false);

        // Filter available credentials
        let available: Vec<_> = entries
            .iter()
            .filter(|e| {
                if e.disabled {
                    return false;
                }
                // In balanced mode, filter out FREE accounts for Opus requests
                if mode == "balanced" && is_opus && !e.credentials.supports_opus() {
                    return false;
                }
                true
            })
            .collect();

        if available.is_empty() {
            return None;
        }

        match mode {
            "balanced" => {
                // Least-Used strategy: Select credential with fewest successes
                // Tie-breaker by priority (lower number = higher priority)
                let entry = available
                    .iter()
                    .min_by_key(|e| (e.success_count, e.credentials.priority))?;

                Some((entry.id, entry.credentials.clone()))
            }
            _ => {
                // priority mode (default): Select highest priority
                let entry = available.iter().min_by_key(|e| e.credentials.priority)?;
                Some((entry.id, entry.credentials.clone()))
            }
        }
    }

    /// Get API call context
    ///
    /// Returns call context bound with id, credentials and token
    /// Ensures consistent credential information throughout the API call
    ///
    /// Automatically refreshes if Token is expired or about to expire
    /// On Token refresh failure, tries next available credential (not counted as failure)
    ///
    /// If model is provided and contains "opus", FREE tier accounts will be filtered out in balanced mode
    pub async fn acquire_context(&self, model: Option<&str>) -> anyhow::Result<CallContext> {
        let total = self.total_count();
        let mut tried_count = 0;

        loop {
            if tried_count >= total {
                anyhow::bail!(
                    "Unable to get valid Token from any credential (available: {}/{})",
                    self.available_count(),
                    total
                );
            }

            let (id, credentials) = {
                let is_balanced = self.load_balancing_mode.lock().as_str() == "balanced";

                // balanced mode: Round-robin select for each request, don't fix current_id
                // priority mode: Prefer credential pointed by current_id
                let current_hit = if is_balanced {
                    None
                } else {
                    let entries = self.entries.lock();
                    let current_id = *self.current_id.lock();
                    entries
                        .iter()
                        .find(|e| e.id == current_id && !e.disabled)
                        .map(|e| (e.id, e.credentials.clone()))
                };

                if let Some(hit) = current_hit {
                    hit
                } else {
                    // Current credential unavailable or balanced mode, select based on load balancing strategy
                    let mut best = self.select_next_credential(model);

                    // No available credentials: if "all disabled due to auto-disable", do self-healing similar to restart
                    if best.is_none() {
                        let mut entries = self.entries.lock();
                        if entries.iter().any(|e| {
                            e.disabled && e.disabled_reason == Some(DisabledReason::TooManyFailures)
                        }) {
                            tracing::warn!(
                                "All credentials have been auto-disabled, performing self-healing: reset failure counts and re-enable (equivalent to restart)"
                            );
                            for e in entries.iter_mut() {
                                if e.disabled_reason == Some(DisabledReason::TooManyFailures) {
                                    e.disabled = false;
                                    e.disabled_reason = None;
                                    e.failure_count = 0;
                                }
                            }
                            drop(entries);
                            best = self.select_next_credential(model);
                        }
                    }

                    if let Some((new_id, new_creds)) = best {
                        // Update current_id
                        let mut current_id = self.current_id.lock();
                        *current_id = new_id;
                        (new_id, new_creds)
                    } else {
                        let entries = self.entries.lock();
                        // Note: must calculate available_count before bail!,
                        // because available_count() will try to acquire entries lock,
                        // and we already hold that lock, which would cause deadlock
                        let available = entries.iter().filter(|e| !e.disabled).count();
                        anyhow::bail!("All credentials are disabled ({}/{})", available, total);
                    }
                }
            };

            // Try to get/refresh Token
            match self.try_ensure_token(id, &credentials).await {
                Ok(ctx) => {
                    return Ok(ctx);
                }
                Err(e) => {
                    tracing::warn!("Credential #{} Token refresh failed, trying next credential: {}", id, e);

                    // Token refresh failed, switch to next priority credential (not counted as failure)
                    self.switch_to_next_by_priority();
                    tried_count += 1;
                }
            }
        }
    }

    /// Switch to next highest priority available credential (internal method)
    fn switch_to_next_by_priority(&self) {
        let entries = self.entries.lock();
        let mut current_id = self.current_id.lock();

        // Select highest priority non-disabled credential (excluding current credential)
        if let Some(entry) = entries
            .iter()
            .filter(|e| !e.disabled && e.id != *current_id)
            .min_by_key(|e| e.credentials.priority)
        {
            *current_id = entry.id;
            tracing::info!(
                "Switched to credential #{} (priority {})",
                entry.id,
                entry.credentials.priority
            );
        }
    }

    /// Select highest priority non-disabled credential as current credential (internal method)
    ///
    /// Unlike `switch_to_next_by_priority`, this method does not exclude current credential,
    /// purely selects by priority, used for immediate effect after priority change
    fn select_highest_priority(&self) {
        let entries = self.entries.lock();
        let mut current_id = self.current_id.lock();

        // Select highest priority non-disabled credential (not excluding current credential)
        if let Some(best) = entries
            .iter()
            .filter(|e| !e.disabled)
            .min_by_key(|e| e.credentials.priority)
        {
            if best.id != *current_id {
                tracing::info!(
                    "Switched credential after priority change: #{} -> #{} (priority {})",
                    *current_id,
                    best.id,
                    best.credentials.priority
                );
                *current_id = best.id;
            }
        }
    }

    /// Try to get valid Token using specified credential
    ///
    /// Uses double-checked locking pattern to ensure only one refresh operation at a time
    ///
    /// # Arguments
    /// * `id` - Credential ID, used to update correct entry
    /// * `credentials` - Credential information
    async fn try_ensure_token(
        &self,
        id: u64,
        credentials: &KiroCredentials,
    ) -> anyhow::Result<CallContext> {
        // First check (no lock): Quick check if refresh is needed
        let needs_refresh = is_token_expired(credentials) || is_token_expiring_soon(credentials);

        let creds = if needs_refresh {
            // Acquire refresh lock to ensure only one refresh operation at a time
            let _guard = self.refresh_lock.lock().await;

            // Second check: Re-read credentials after acquiring lock, as other requests may have completed refresh
            let current_creds = {
                let entries = self.entries.lock();
                entries
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| e.credentials.clone())
                    .ok_or_else(|| anyhow::anyhow!("Credential #{} does not exist", id))?
            };

            if is_token_expired(&current_creds) || is_token_expiring_soon(&current_creds) {
                // Actually need to refresh
                let new_creds =
                    refresh_token(&current_creds, &self.config, self.proxy.as_ref()).await?;

                if is_token_expired(&new_creds) {
                    anyhow::bail!("Refreshed Token is still invalid or expired");
                }

                // Update credentials
                {
                    let mut entries = self.entries.lock();
                    if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
                        entry.credentials = new_creds.clone();
                    }
                }

                // Write back credentials to file (only for multiple credentials format), log warning on failure
                if let Err(e) = self.persist_credentials() {
                    tracing::warn!("Failed to persist after Token refresh (does not affect this request): {}", e);
                }

                new_creds
            } else {
                // Other request already completed refresh, use new credentials directly
                tracing::debug!("Token already refreshed by another request, skipping refresh");
                current_creds
            }
        } else {
            credentials.clone()
        };

        let token = creds
            .access_token
            .clone()
            .ok_or_else(|| anyhow::anyhow!("No available accessToken"))?;

        Ok(CallContext {
            id,
            credentials: creds,
            token,
        })
    }

    /// Write credentials list back to source file
    ///
    /// Only writes back when the following conditions are met:
    /// - Source file is multiple credentials format (array)
    /// - credentials_path is set
    ///
    /// # Returns
    /// - `Ok(true)` - Successfully wrote to file
    /// - `Ok(false)` - Skipped write (not multiple credentials format or no path configured)
    /// - `Err(_)` - Write failed
    fn persist_credentials(&self) -> anyhow::Result<bool> {
        use anyhow::Context;

        // Only write back for multiple credentials format
        if !*self.is_multiple_format.lock() {
            return Ok(false);
        }

        let path = match &self.credentials_path {
            Some(p) => p,
            None => return Ok(false),
        };

        // Collect all credentials
        let credentials: Vec<KiroCredentials> = {
            let entries = self.entries.lock();
            entries
                .iter()
                .map(|e| {
                    let mut cred = e.credentials.clone();
                    cred.canonicalize_auth_method();
                    cred
                })
                .collect()
        };

        // Serialize to pretty JSON
        let json = serde_json::to_string_pretty(&credentials).context("Failed to serialize credentials")?;

        // Write to file (use block_in_place in Tokio runtime to avoid blocking worker)
        if tokio::runtime::Handle::try_current().is_ok() {
            tokio::task::block_in_place(|| std::fs::write(path, &json))
                .with_context(|| format!("Failed to write back credentials file: {:?}", path))?;
        } else {
            std::fs::write(path, &json).with_context(|| format!("Failed to write back credentials file: {:?}", path))?;
        }

        tracing::debug!("Wrote back credentials to file: {:?}", path);
        Ok(true)
    }

    /// Get cache directory (directory containing credentials file)
    pub fn cache_dir(&self) -> Option<PathBuf> {
        self.credentials_path
            .as_ref()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    }

    /// Statistics data file path
    fn stats_path(&self) -> Option<PathBuf> {
        self.cache_dir().map(|d| d.join("kiro_stats.json"))
    }

    /// Load statistics data from disk and apply to current entries
    fn load_stats(&self) {
        let path = match self.stats_path() {
            Some(p) => p,
            None => return,
        };

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => return, // File doesn't exist on first run
        };

        let stats: HashMap<String, StatsEntry> = match serde_json::from_str(&content) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to parse statistics cache, will ignore: {}", e);
                return;
            }
        };

        let mut entries = self.entries.lock();
        for entry in entries.iter_mut() {
            if let Some(s) = stats.get(&entry.id.to_string()) {
                entry.success_count = s.success_count;
                entry.last_used_at = s.last_used_at.clone();
            }
        }
        *self.last_stats_save_at.lock() = Some(Instant::now());
        self.stats_dirty.store(false, Ordering::Relaxed);
        tracing::info!("Loaded {} statistics entries from cache", stats.len());
    }

    /// Persist current statistics data to disk
    fn save_stats(&self) {
        let path = match self.stats_path() {
            Some(p) => p,
            None => return,
        };

        let stats: HashMap<String, StatsEntry> = {
            let entries = self.entries.lock();
            entries
                .iter()
                .map(|e| {
                    (
                        e.id.to_string(),
                        StatsEntry {
                            success_count: e.success_count,
                            last_used_at: e.last_used_at.clone(),
                        },
                    )
                })
                .collect()
        };

        match serde_json::to_string_pretty(&stats) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    tracing::warn!("Failed to save statistics cache: {}", e);
                } else {
                    *self.last_stats_save_at.lock() = Some(Instant::now());
                    self.stats_dirty.store(false, Ordering::Relaxed);
                }
            }
            Err(e) => tracing::warn!("Failed to serialize statistics data: {}", e),
        }
    }

    /// Mark statistics data as updated, and decide whether to flush immediately based on debounce strategy
    fn save_stats_debounced(&self) {
        self.stats_dirty.store(true, Ordering::Relaxed);

        let should_flush = {
            let last = *self.last_stats_save_at.lock();
            match last {
                Some(last_saved_at) => last_saved_at.elapsed() >= STATS_SAVE_DEBOUNCE,
                None => true,
            }
        };

        if should_flush {
            self.save_stats();
        }
    }

    /// Report specified credential API call success
    ///
    /// Resets the credential's failure count
    ///
    /// # Arguments
    /// * `id` - Credential ID (from CallContext)
    pub fn report_success(&self, id: u64) {
        {
            let mut entries = self.entries.lock();
            if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
                entry.failure_count = 0;
                entry.success_count += 1;
                entry.last_used_at = Some(Utc::now().to_rfc3339());
                tracing::debug!(
                    "Credential #{} API call succeeded (total {} times)",
                    id,
                    entry.success_count
                );
            }
        }
        self.save_stats_debounced();
    }

    /// Report specified credential API call failure
    ///
    /// Increments failure count, disables credential and switches to highest priority available credential when threshold reached
    /// Returns whether there are still available credentials to retry
    ///
    /// # Arguments
    /// * `id` - Credential ID (from CallContext)
    pub fn report_failure(&self, id: u64) -> bool {
        let result = {
            let mut entries = self.entries.lock();
            let mut current_id = self.current_id.lock();

            let entry = match entries.iter_mut().find(|e| e.id == id) {
                Some(e) => e,
                None => return entries.iter().any(|e| !e.disabled),
            };

            entry.failure_count += 1;
            entry.last_used_at = Some(Utc::now().to_rfc3339());
            let failure_count = entry.failure_count;

            tracing::warn!(
                "Credential #{} API call failed ({}/{})",
                id,
                failure_count,
                MAX_FAILURES_PER_CREDENTIAL
            );

            if failure_count >= MAX_FAILURES_PER_CREDENTIAL {
                entry.disabled = true;
                entry.disabled_reason = Some(DisabledReason::TooManyFailures);
                tracing::error!("Credential #{} has failed {} consecutive times, disabled", id, failure_count);

                // Switch to highest priority available credential
                if let Some(next) = entries
                    .iter()
                    .filter(|e| !e.disabled)
                    .min_by_key(|e| e.credentials.priority)
                {
                    *current_id = next.id;
                    tracing::info!(
                        "Switched to credential #{} (priority {})",
                        next.id,
                        next.credentials.priority
                    );
                } else {
                    tracing::error!("All credentials are disabled!");
                }
            }

            entries.iter().any(|e| !e.disabled)
        };
        self.save_stats_debounced();
        result
    }

    /// Report specified credential quota exhausted
    ///
    /// Used to handle 402 Payment Required with reason `MONTHLY_REQUEST_COUNT`:
    /// - Immediately disable the credential (don't wait for consecutive failure threshold)
    /// - Switch to next available credential to continue retry
    /// - Return whether there are still available credentials
    pub fn report_quota_exhausted(&self, id: u64) -> bool {
        let result = {
            let mut entries = self.entries.lock();
            let mut current_id = self.current_id.lock();

            let entry = match entries.iter_mut().find(|e| e.id == id) {
                Some(e) => e,
                None => return entries.iter().any(|e| !e.disabled),
            };

            if entry.disabled {
                return entries.iter().any(|e| !e.disabled);
            }

            entry.disabled = true;
            entry.disabled_reason = Some(DisabledReason::QuotaExceeded);
            entry.last_used_at = Some(Utc::now().to_rfc3339());
            // Set to threshold for intuitive display in admin panel that credential is unavailable
            entry.failure_count = MAX_FAILURES_PER_CREDENTIAL;

            tracing::error!("Credential #{} quota exhausted (MONTHLY_REQUEST_COUNT), disabled", id);

            // Switch to highest priority available credential
            if let Some(next) = entries
                .iter()
                .filter(|e| !e.disabled)
                .min_by_key(|e| e.credentials.priority)
            {
                *current_id = next.id;
                tracing::info!(
                    "Switched to credential #{} (priority {})",
                    next.id,
                    next.credentials.priority
                );
                true
            } else {
                tracing::error!("All credentials are disabled!");
                false
            }
        };
        self.save_stats_debounced();
        result
    }

    /// Switch to highest priority available credential
    ///
    /// Returns whether switch was successful
    pub fn switch_to_next(&self) -> bool {
        let entries = self.entries.lock();
        let mut current_id = self.current_id.lock();

        // Select highest priority non-disabled credential (excluding current credential)
        if let Some(next) = entries
            .iter()
            .filter(|e| !e.disabled && e.id != *current_id)
            .min_by_key(|e| e.credentials.priority)
        {
            *current_id = next.id;
            tracing::info!(
                "Switched to credential #{} (priority {})",
                next.id,
                next.credentials.priority
            );
            true
        } else {
            // No other available credentials, check if current credential is available
            entries.iter().any(|e| e.id == *current_id && !e.disabled)
        }
    }

    /// Get usage limits information
    pub async fn get_usage_limits(&self) -> anyhow::Result<UsageLimitsResponse> {
        let ctx = self.acquire_context(None).await?;
        get_usage_limits(
            &ctx.credentials,
            &self.config,
            &ctx.token,
            self.proxy.as_ref(),
        )
        .await
    }

    // ========================================================================
    // Admin API methods
    // ========================================================================

    /// Get manager state snapshot (for Admin API)
    pub fn snapshot(&self) -> ManagerSnapshot {
        let entries = self.entries.lock();
        let current_id = *self.current_id.lock();
        let available = entries.iter().filter(|e| !e.disabled).count();

        ManagerSnapshot {
            entries: entries
                .iter()
                .map(|e| CredentialEntrySnapshot {
                    id: e.id,
                    priority: e.credentials.priority,
                    disabled: e.disabled,
                    failure_count: e.failure_count,
                    auth_method: e.credentials.auth_method.as_deref().map(|m| {
                        if m.eq_ignore_ascii_case("builder-id") || m.eq_ignore_ascii_case("iam") {
                            "idc".to_string()
                        } else {
                            m.to_string()
                        }
                    }),
                    has_profile_arn: e.credentials.profile_arn.is_some(),
                    expires_at: e.credentials.expires_at.clone(),
                    refresh_token_hash: e.credentials.refresh_token.as_deref().map(sha256_hex),
                    email: e.credentials.email.clone(),
                    success_count: e.success_count,
                    last_used_at: e.last_used_at.clone(),
                })
                .collect(),
            current_id,
            total: entries.len(),
            available,
        }
    }

    /// Set credential disabled status (Admin API)
    pub fn set_disabled(&self, id: u64, disabled: bool) -> anyhow::Result<()> {
        {
            let mut entries = self.entries.lock();
            let entry = entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or_else(|| anyhow::anyhow!("Credential does not exist: {}", id))?;
            entry.disabled = disabled;
            if !disabled {
                // Reset failure count when enabling
                entry.failure_count = 0;
                entry.disabled_reason = None;
            } else {
                entry.disabled_reason = Some(DisabledReason::Manual);
            }
        }
        // Persist changes
        self.persist_credentials()?;
        Ok(())
    }

    /// Set credential priority (Admin API)
    ///
    /// After modifying priority, immediately re-selects current credential based on new priority.
    /// Even if persistence fails, priority and current credential selection in memory will take effect.
    pub fn set_priority(&self, id: u64, priority: u32) -> anyhow::Result<()> {
        {
            let mut entries = self.entries.lock();
            let entry = entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or_else(|| anyhow::anyhow!("Credential does not exist: {}", id))?;
            entry.credentials.priority = priority;
        }
        // Immediately re-select current credential based on new priority (regardless of persistence success)
        self.select_highest_priority();
        // Persist changes
        self.persist_credentials()?;
        Ok(())
    }

    /// Reset credential failure count and re-enable (Admin API)
    pub fn reset_and_enable(&self, id: u64) -> anyhow::Result<()> {
        {
            let mut entries = self.entries.lock();
            let entry = entries
                .iter_mut()
                .find(|e| e.id == id)
                .ok_or_else(|| anyhow::anyhow!("Credential does not exist: {}", id))?;
            entry.failure_count = 0;
            entry.disabled = false;
            entry.disabled_reason = None;
        }
        // Persist changes
        self.persist_credentials()?;
        Ok(())
    }

    /// Get usage limits for specified credential (Admin API)
    pub async fn get_usage_limits_for(&self, id: u64) -> anyhow::Result<UsageLimitsResponse> {
        let credentials = {
            let entries = self.entries.lock();
            entries
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.credentials.clone())
                .ok_or_else(|| anyhow::anyhow!("Credential does not exist: {}", id))?
        };

        // Check if token needs refresh
        let needs_refresh = is_token_expired(&credentials) || is_token_expiring_soon(&credentials);

        let token = if needs_refresh {
            let _guard = self.refresh_lock.lock().await;
            let current_creds = {
                let entries = self.entries.lock();
                entries
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| e.credentials.clone())
                    .ok_or_else(|| anyhow::anyhow!("Credential does not exist: {}", id))?
            };

            if is_token_expired(&current_creds) || is_token_expiring_soon(&current_creds) {
                let new_creds =
                    refresh_token(&current_creds, &self.config, self.proxy.as_ref()).await?;
                {
                    let mut entries = self.entries.lock();
                    if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
                        entry.credentials = new_creds.clone();
                    }
                }
                // Log warning on persistence failure, does not affect this request
                if let Err(e) = self.persist_credentials() {
                    tracing::warn!("Failed to persist after Token refresh (does not affect this request): {}", e);
                }
                new_creds
                    .access_token
                    .ok_or_else(|| anyhow::anyhow!("No access_token after refresh"))?
            } else {
                current_creds
                    .access_token
                    .ok_or_else(|| anyhow::anyhow!("Credential has no access_token"))?
            }
        } else {
            credentials
                .access_token
                .ok_or_else(|| anyhow::anyhow!("Credential has no access_token"))?
        };

        let credentials = {
            let entries = self.entries.lock();
            entries
                .iter()
                .find(|e| e.id == id)
                .map(|e| e.credentials.clone())
                .ok_or_else(|| anyhow::anyhow!("Credential does not exist: {}", id))?
        };

        let usage = get_usage_limits(&credentials, &self.config, &token, self.proxy.as_ref()).await?;

        // Update subscription_title in credential if available
        if let Some(title) = usage.subscription_title() {
            let mut entries = self.entries.lock();
            if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
                if entry.credentials.subscription_title.as_deref() != Some(title) {
                    entry.credentials.subscription_title = Some(title.to_string());
                    drop(entries);
                    // Persist to config file
                    if let Err(e) = self.persist_credentials() {
                        tracing::warn!("Failed to persist subscription_title: {}", e);
                    }
                }
            }
        }

        Ok(usage)
    }

    /// Add new credential (Admin API)
    ///
    /// # Flow
    /// 1. Validate basic credential fields (refresh_token not empty)
    /// 2. Detect duplicates based on SHA-256 hash of refreshToken
    /// 3. Try to refresh Token to validate credential
    /// 4. Assign new ID (current max ID + 1)
    /// 5. Add to entries list
    /// 6. Persist to config file
    ///
    /// # Returns
    /// - `Ok(u64)` - New credential ID
    /// - `Err(_)` - Validation failed or add failed
    pub async fn add_credential(&self, new_cred: KiroCredentials) -> anyhow::Result<u64> {
        // 1. Basic validation
        validate_refresh_token(&new_cred)?;

        // 2. Detect duplicates based on SHA-256 hash of refreshToken
        let new_refresh_token = new_cred
            .refresh_token
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Missing refreshToken"))?;
        let new_refresh_token_hash = sha256_hex(new_refresh_token);
        let duplicate_exists = {
            let entries = self.entries.lock();
            entries.iter().any(|entry| {
                entry
                    .credentials
                    .refresh_token
                    .as_deref()
                    .map(sha256_hex)
                    .as_deref()
                    == Some(new_refresh_token_hash.as_str())
            })
        };
        if duplicate_exists {
            anyhow::bail!("Credential already exists (duplicate refreshToken)");
        }

        // 3. Try to refresh Token to validate credential
        let mut validated_cred =
            refresh_token(&new_cred, &self.config, self.proxy.as_ref()).await?;

        // 4. Assign new ID
        let new_id = {
            let entries = self.entries.lock();
            entries.iter().map(|e| e.id).max().unwrap_or(0) + 1
        };

        // 5. Set ID and preserve user input metadata
        validated_cred.id = Some(new_id);
        validated_cred.priority = new_cred.priority;
        validated_cred.auth_method = new_cred.auth_method.map(|m| {
            if m.eq_ignore_ascii_case("builder-id") || m.eq_ignore_ascii_case("iam") {
                "idc".to_string()
            } else {
                m
            }
        });
        validated_cred.client_id = new_cred.client_id;
        validated_cred.client_secret = new_cred.client_secret;
        validated_cred.region = new_cred.region;
        validated_cred.auth_region = new_cred.auth_region;
        validated_cred.api_region = new_cred.api_region;
        validated_cred.machine_id = new_cred.machine_id;
        validated_cred.email = new_cred.email;

        {
            let mut entries = self.entries.lock();
            entries.push(CredentialEntry {
                id: new_id,
                credentials: validated_cred,
                failure_count: 0,
                disabled: false,
                disabled_reason: None,
                success_count: 0,
                last_used_at: None,
            });
        }

        // 6. Upgrade to multiple credentials format and persist
        {
            let mut is_multiple = self.is_multiple_format.lock();
            if !*is_multiple {
                *is_multiple = true;
                tracing::info!("Credential format upgraded to multiple credentials array format");
            }
        }
        self.persist_credentials()?;

        tracing::info!("Successfully added credential #{}", new_id);
        Ok(new_id)
    }

    /// Delete credential (Admin API)
    ///
    /// # Preconditions
    /// - Credential must be disabled (disabled = true)
    ///
    /// # Behavior
    /// 1. Verify credential exists
    /// 2. Verify credential is disabled
    /// 3. Remove from entries
    /// 4. If deleted credential was current, switch to highest priority available credential
    /// 5. If no credentials remain after deletion, reset current_id to 0
    /// 6. Persist to file
    ///
    /// # Returns
    /// - `Ok(())` - Delete successful
    /// - `Err(_)` - Credential does not exist, not disabled, or persistence failed
    pub fn delete_credential(&self, id: u64) -> anyhow::Result<()> {
        let was_current = {
            let mut entries = self.entries.lock();

            // Find credential
            let entry = entries
                .iter()
                .find(|e| e.id == id)
                .ok_or_else(|| anyhow::anyhow!("Credential does not exist: {}", id))?;

            // Check if disabled
            if !entry.disabled {
                anyhow::bail!("Can only delete disabled credentials (please disable credential #{} first)", id);
            }

            // Record if it's current credential
            let current_id = *self.current_id.lock();
            let was_current = current_id == id;

            // Delete credential
            entries.retain(|e| e.id != id);

            was_current
        };

        // If deleted credential was current, switch to highest priority available credential
        if was_current {
            self.select_highest_priority();
        }

        // If no credentials remain after deletion, reset current_id to 0 (consistent with initialization behavior)
        {
            let entries = self.entries.lock();
            if entries.is_empty() {
                let mut current_id = self.current_id.lock();
                *current_id = 0;
                tracing::info!("All credentials deleted, current_id reset to 0");
            }
        }

        // Persist changes
        self.persist_credentials()?;

        tracing::info!("Deleted credential #{}", id);
        Ok(())
    }

    /// Get load balancing mode (Admin API)
    pub fn get_load_balancing_mode(&self) -> String {
        self.load_balancing_mode.lock().clone()
    }

    fn persist_load_balancing_mode(&self, mode: &str) -> anyhow::Result<()> {
        use anyhow::Context;

        let config_path = match self.config.config_path() {
            Some(path) => path.to_path_buf(),
            None => {
                tracing::warn!("Config file path unknown, load balancing mode only effective in current process: {}", mode);
                return Ok(());
            }
        };

        let mut config = Config::load(&config_path)
            .with_context(|| format!("Failed to reload config: {}", config_path.display()))?;
        config.load_balancing_mode = mode.to_string();
        config
            .save()
            .with_context(|| format!("Failed to persist load balancing mode: {}", config_path.display()))?;

        Ok(())
    }

    /// Set load balancing mode (Admin API)
    pub fn set_load_balancing_mode(&self, mode: String) -> anyhow::Result<()> {
        // Validate mode value
        if mode != "priority" && mode != "balanced" {
            anyhow::bail!("Invalid load balancing mode: {}", mode);
        }

        let previous_mode = self.get_load_balancing_mode();
        if previous_mode == mode {
            return Ok(());
        }

        *self.load_balancing_mode.lock() = mode.clone();

        if let Err(err) = self.persist_load_balancing_mode(&mode) {
            *self.load_balancing_mode.lock() = previous_mode;
            return Err(err);
        }

        tracing::info!("Load balancing mode set to: {}", mode);
        Ok(())
    }
}

impl Drop for MultiTokenManager {
    fn drop(&mut self) {
        if self.stats_dirty.load(Ordering::Relaxed) {
            self.save_stats();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_manager_new() {
        let config = Config::default();
        let credentials = KiroCredentials::default();
        let tm = TokenManager::new(config, credentials, None);
        assert!(tm.credentials().access_token.is_none());
    }

    #[test]
    fn test_is_token_expired_with_expired_token() {
        let mut credentials = KiroCredentials::default();
        credentials.expires_at = Some("2020-01-01T00:00:00Z".to_string());
        assert!(is_token_expired(&credentials));
    }

    #[test]
    fn test_is_token_expired_with_valid_token() {
        let mut credentials = KiroCredentials::default();
        let future = Utc::now() + Duration::hours(1);
        credentials.expires_at = Some(future.to_rfc3339());
        assert!(!is_token_expired(&credentials));
    }

    #[test]
    fn test_is_token_expired_within_5_minutes() {
        let mut credentials = KiroCredentials::default();
        let expires = Utc::now() + Duration::minutes(3);
        credentials.expires_at = Some(expires.to_rfc3339());
        assert!(is_token_expired(&credentials));
    }

    #[test]
    fn test_is_token_expired_no_expires_at() {
        let credentials = KiroCredentials::default();
        assert!(is_token_expired(&credentials));
    }

    #[test]
    fn test_is_token_expiring_soon_within_10_minutes() {
        let mut credentials = KiroCredentials::default();
        let expires = Utc::now() + Duration::minutes(8);
        credentials.expires_at = Some(expires.to_rfc3339());
        assert!(is_token_expiring_soon(&credentials));
    }

    #[test]
    fn test_is_token_expiring_soon_beyond_10_minutes() {
        let mut credentials = KiroCredentials::default();
        let expires = Utc::now() + Duration::minutes(15);
        credentials.expires_at = Some(expires.to_rfc3339());
        assert!(!is_token_expiring_soon(&credentials));
    }

    #[test]
    fn test_validate_refresh_token_missing() {
        let credentials = KiroCredentials::default();
        let result = validate_refresh_token(&credentials);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_refresh_token_valid() {
        let mut credentials = KiroCredentials::default();
        credentials.refresh_token = Some("a".repeat(150));
        let result = validate_refresh_token(&credentials);
        assert!(result.is_ok());
    }

    #[test]
    fn test_sha256_hex() {
        let result = sha256_hex("test");
        assert_eq!(
            result,
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }

    #[tokio::test]
    async fn test_add_credential_reject_duplicate_refresh_token() {
        let config = Config::default();

        let mut existing = KiroCredentials::default();
        existing.refresh_token = Some("a".repeat(150));

        let manager = MultiTokenManager::new(config, vec![existing], None, None, false).unwrap();

        let mut duplicate = KiroCredentials::default();
        duplicate.refresh_token = Some("a".repeat(150));

        let result = manager.add_credential(duplicate).await;
        assert!(result.is_err());
        assert!(result.err().unwrap().to_string().contains("Credential already exists"));
    }

    // MultiTokenManager tests

    #[test]
    fn test_multi_token_manager_new() {
        let config = Config::default();
        let mut cred1 = KiroCredentials::default();
        cred1.priority = 0;
        let mut cred2 = KiroCredentials::default();
        cred2.priority = 1;

        let manager =
            MultiTokenManager::new(config, vec![cred1, cred2], None, None, false).unwrap();
        assert_eq!(manager.total_count(), 2);
        assert_eq!(manager.available_count(), 2);
    }

    #[test]
    fn test_multi_token_manager_empty_credentials() {
        let config = Config::default();
        let result = MultiTokenManager::new(config, vec![], None, None, false);
        // Supports starting with 0 credentials (can add via admin panel)
        assert!(result.is_ok());
        let manager = result.unwrap();
        assert_eq!(manager.total_count(), 0);
        assert_eq!(manager.available_count(), 0);
    }

    #[test]
    fn test_multi_token_manager_duplicate_ids() {
        let config = Config::default();
        let mut cred1 = KiroCredentials::default();
        cred1.id = Some(1);
        let mut cred2 = KiroCredentials::default();
        cred2.id = Some(1); // Duplicate ID

        let result = MultiTokenManager::new(config, vec![cred1, cred2], None, None, false);
        assert!(result.is_err());
        let err_msg = result.err().unwrap().to_string();
        assert!(
            err_msg.contains("Duplicate credential IDs"),
            "Error message should contain 'Duplicate credential IDs', actual: {}",
            err_msg
        );
    }

    #[test]
    fn test_multi_token_manager_report_failure() {
        let config = Config::default();
        let cred1 = KiroCredentials::default();
        let cred2 = KiroCredentials::default();

        let manager =
            MultiTokenManager::new(config, vec![cred1, cred2], None, None, false).unwrap();

        // Credentials will be auto-assigned IDs (starting from 1)
        // First two failures won't disable (using ID 1)
        assert!(manager.report_failure(1));
        assert!(manager.report_failure(1));
        assert_eq!(manager.available_count(), 2);

        // Third failure will disable first credential
        assert!(manager.report_failure(1));
        assert_eq!(manager.available_count(), 1);

        // Continue failing second credential (using ID 2)
        assert!(manager.report_failure(2));
        assert!(manager.report_failure(2));
        assert!(!manager.report_failure(2)); // All credentials disabled
        assert_eq!(manager.available_count(), 0);
    }

    #[test]
    fn test_multi_token_manager_report_success() {
        let config = Config::default();
        let cred = KiroCredentials::default();

        let manager = MultiTokenManager::new(config, vec![cred], None, None, false).unwrap();

        // Fail twice (using ID 1)
        manager.report_failure(1);
        manager.report_failure(1);

        // Success resets count (using ID 1)
        manager.report_success(1);

        // Two more failures won't disable
        manager.report_failure(1);
        manager.report_failure(1);
        assert_eq!(manager.available_count(), 1);
    }

    #[test]
    fn test_multi_token_manager_switch_to_next() {
        let config = Config::default();
        let mut cred1 = KiroCredentials::default();
        cred1.refresh_token = Some("token1".to_string());
        let mut cred2 = KiroCredentials::default();
        cred2.refresh_token = Some("token2".to_string());

        let manager =
            MultiTokenManager::new(config, vec![cred1, cred2], None, None, false).unwrap();

        // Initial is first credential
        assert_eq!(
            manager.credentials().refresh_token,
            Some("token1".to_string())
        );

        // Switch to next
        assert!(manager.switch_to_next());
        assert_eq!(
            manager.credentials().refresh_token,
            Some("token2".to_string())
        );
    }

    #[test]
    fn test_set_load_balancing_mode_persists_to_config_file() {
        let config_path = std::env::temp_dir().join(format!(
            "kiro-load-balancing-{}.json",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&config_path, r#"{"loadBalancingMode":"priority"}"#).unwrap();

        let config = Config::load(&config_path).unwrap();
        let manager = MultiTokenManager::new(
            config,
            vec![KiroCredentials::default()],
            None,
            None,
            false,
        )
        .unwrap();

        manager
            .set_load_balancing_mode("balanced".to_string())
            .unwrap();

        let persisted = Config::load(&config_path).unwrap();
        assert_eq!(persisted.load_balancing_mode, "balanced");
        assert_eq!(manager.get_load_balancing_mode(), "balanced");

        std::fs::remove_file(&config_path).unwrap();
    }

    #[tokio::test]
    async fn test_multi_token_manager_acquire_context_auto_recovers_all_disabled() {
        let config = Config::default();
        let mut cred1 = KiroCredentials::default();
        cred1.access_token = Some("t1".to_string());
        cred1.expires_at = Some((Utc::now() + Duration::hours(1)).to_rfc3339());
        let mut cred2 = KiroCredentials::default();
        cred2.access_token = Some("t2".to_string());
        cred2.expires_at = Some((Utc::now() + Duration::hours(1)).to_rfc3339());

        let manager =
            MultiTokenManager::new(config, vec![cred1, cred2], None, None, false).unwrap();

        // Credentials will be auto-assigned IDs (starting from 1)
        for _ in 0..MAX_FAILURES_PER_CREDENTIAL {
            manager.report_failure(1);
        }
        for _ in 0..MAX_FAILURES_PER_CREDENTIAL {
            manager.report_failure(2);
        }

        assert_eq!(manager.available_count(), 0);

        // Should trigger self-healing: reset failure counts and re-enable, avoiding need to restart process
        let ctx = manager.acquire_context(None).await.unwrap();
        assert!(ctx.token == "t1" || ctx.token == "t2");
        assert_eq!(manager.available_count(), 2);
    }

    #[test]
    fn test_multi_token_manager_report_quota_exhausted() {
        let config = Config::default();
        let cred1 = KiroCredentials::default();
        let cred2 = KiroCredentials::default();

        let manager =
            MultiTokenManager::new(config, vec![cred1, cred2], None, None, false).unwrap();

        // Credentials will be auto-assigned IDs (starting from 1)
        assert_eq!(manager.available_count(), 2);
        assert!(manager.report_quota_exhausted(1));
        assert_eq!(manager.available_count(), 1);

        // After disabling second, no available credentials
        assert!(!manager.report_quota_exhausted(2));
        assert_eq!(manager.available_count(), 0);
    }

    #[tokio::test]
    async fn test_multi_token_manager_quota_disabled_is_not_auto_recovered() {
        let config = Config::default();
        let cred1 = KiroCredentials::default();
        let cred2 = KiroCredentials::default();

        let manager =
            MultiTokenManager::new(config, vec![cred1, cred2], None, None, false).unwrap();

        manager.report_quota_exhausted(1);
        manager.report_quota_exhausted(2);
        assert_eq!(manager.available_count(), 0);

        let err = manager.acquire_context(None).await.err().unwrap().to_string();
        assert!(
            err.contains("All credentials are disabled"),
            "Error should indicate all credentials disabled, actual: {}",
            err
        );
        assert_eq!(manager.available_count(), 0);
    }

    // ============ Credential-level Region priority tests ============

    #[test]
    fn test_credential_region_priority_uses_credential_auth_region() {
        // When credential has auth_region configured, should use credential's auth_region
        let mut config = Config::default();
        config.region = "us-west-2".to_string();

        let mut credentials = KiroCredentials::default();
        credentials.auth_region = Some("eu-west-1".to_string());

        let region = credentials.effective_auth_region(&config);
        assert_eq!(region, "eu-west-1");
    }

    #[test]
    fn test_credential_region_priority_fallback_to_credential_region() {
        // When credential has no auth_region but has region configured, should fall back to credential.region
        let mut config = Config::default();
        config.region = "us-west-2".to_string();

        let mut credentials = KiroCredentials::default();
        credentials.region = Some("eu-central-1".to_string());

        let region = credentials.effective_auth_region(&config);
        assert_eq!(region, "eu-central-1");
    }

    #[test]
    fn test_credential_region_priority_fallback_to_config() {
        // When credential has no auth_region and no region configured, should fall back to config
        let mut config = Config::default();
        config.region = "us-west-2".to_string();

        let credentials = KiroCredentials::default();
        assert!(credentials.auth_region.is_none());
        assert!(credentials.region.is_none());

        let region = credentials.effective_auth_region(&config);
        assert_eq!(region, "us-west-2");
    }

    #[test]
    fn test_multiple_credentials_use_respective_regions() {
        // In multi-credential scenario, different credentials use their own auth_region
        let mut config = Config::default();
        config.region = "ap-northeast-1".to_string();

        let mut cred1 = KiroCredentials::default();
        cred1.auth_region = Some("us-east-1".to_string());

        let mut cred2 = KiroCredentials::default();
        cred2.region = Some("eu-west-1".to_string());

        let cred3 = KiroCredentials::default(); // No region, uses config

        assert_eq!(cred1.effective_auth_region(&config), "us-east-1");
        assert_eq!(cred2.effective_auth_region(&config), "eu-west-1");
        assert_eq!(cred3.effective_auth_region(&config), "ap-northeast-1");
    }

    #[test]
    fn test_idc_oidc_endpoint_uses_credential_auth_region() {
        // Verify IdC OIDC endpoint URL uses credential auth_region
        let mut config = Config::default();
        config.region = "us-west-2".to_string();

        let mut credentials = KiroCredentials::default();
        credentials.auth_region = Some("eu-central-1".to_string());

        let region = credentials.effective_auth_region(&config);
        let refresh_url = format!("https://oidc.{}.amazonaws.com/token", region);

        assert_eq!(refresh_url, "https://oidc.eu-central-1.amazonaws.com/token");
    }

    #[test]
    fn test_social_refresh_endpoint_uses_credential_auth_region() {
        // Verify Social refresh endpoint URL uses credential auth_region
        let mut config = Config::default();
        config.region = "us-west-2".to_string();

        let mut credentials = KiroCredentials::default();
        credentials.auth_region = Some("ap-southeast-1".to_string());

        let region = credentials.effective_auth_region(&config);
        let refresh_url = format!("https://prod.{}.auth.desktop.kiro.dev/refreshToken", region);

        assert_eq!(
            refresh_url,
            "https://prod.ap-southeast-1.auth.desktop.kiro.dev/refreshToken"
        );
    }

    #[test]
    fn test_api_call_uses_effective_api_region() {
        // Verify API call uses effective_api_region
        let mut config = Config::default();
        config.region = "us-west-2".to_string();

        let mut credentials = KiroCredentials::default();
        credentials.region = Some("eu-west-1".to_string());

        // credential.region does not participate in api_region fallback chain
        let api_region = credentials.effective_api_region(&config);
        let api_host = format!("q.{}.amazonaws.com", api_region);

        assert_eq!(api_host, "q.us-west-2.amazonaws.com");
    }

    #[test]
    fn test_api_call_uses_credential_api_region() {
        // When credential has api_region configured, API call should use credential's api_region
        let mut config = Config::default();
        config.region = "us-west-2".to_string();

        let mut credentials = KiroCredentials::default();
        credentials.api_region = Some("eu-central-1".to_string());

        let api_region = credentials.effective_api_region(&config);
        let api_host = format!("q.{}.amazonaws.com", api_region);

        assert_eq!(api_host, "q.eu-central-1.amazonaws.com");
    }

    #[test]
    fn test_credential_region_empty_string_treated_as_set() {
        // Empty string auth_region is treated as set (not recommended, but behavior should be consistent)
        let mut config = Config::default();
        config.region = "us-west-2".to_string();

        let mut credentials = KiroCredentials::default();
        credentials.auth_region = Some("".to_string());

        let region = credentials.effective_auth_region(&config);
        // Empty string is treated as set, won't fall back to config
        assert_eq!(region, "");
    }

    #[test]
    fn test_auth_and_api_region_independent() {
        // auth_region and api_region are independent of each other
        let mut config = Config::default();
        config.region = "default".to_string();

        let mut credentials = KiroCredentials::default();
        credentials.auth_region = Some("auth-only".to_string());
        credentials.api_region = Some("api-only".to_string());

        assert_eq!(credentials.effective_auth_region(&config), "auth-only");
        assert_eq!(credentials.effective_api_region(&config), "api-only");
    }
}
