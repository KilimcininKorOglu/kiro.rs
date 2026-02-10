//! Admin API business logic service

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::kiro::model::credentials::KiroCredentials;
use crate::kiro::token_manager::MultiTokenManager;

use super::error::AdminServiceError;
use super::types::{
    AddCredentialRequest, AddCredentialResponse, BalanceResponse, CredentialStatusItem,
    CredentialsStatusResponse, LoadBalancingModeResponse, SetLoadBalancingModeRequest,
};

/// Balance cache expiration time (seconds), 5 minutes
const BALANCE_CACHE_TTL_SECS: i64 = 300;

/// Cached balance entry (with timestamp)
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedBalance {
    /// Cache time (Unix seconds)
    cached_at: f64,
    /// Cached balance data
    data: BalanceResponse,
}

/// Admin service
///
/// Encapsulates all Admin API business logic
pub struct AdminService {
    token_manager: Arc<MultiTokenManager>,
    balance_cache: Mutex<HashMap<u64, CachedBalance>>,
    cache_path: Option<PathBuf>,
}

impl AdminService {
    pub fn new(token_manager: Arc<MultiTokenManager>) -> Self {
        let cache_path = token_manager
            .cache_dir()
            .map(|d| d.join("kiro_balance_cache.json"));

        let balance_cache = Self::load_balance_cache_from(&cache_path);

        Self {
            token_manager,
            balance_cache: Mutex::new(balance_cache),
            cache_path,
        }
    }

    /// Get all credential statuses
    pub fn get_all_credentials(&self) -> CredentialsStatusResponse {
        let snapshot = self.token_manager.snapshot();

        let mut credentials: Vec<CredentialStatusItem> = snapshot
            .entries
            .into_iter()
            .map(|entry| CredentialStatusItem {
                id: entry.id,
                priority: entry.priority,
                disabled: entry.disabled,
                failure_count: entry.failure_count,
                is_current: entry.id == snapshot.current_id,
                expires_at: entry.expires_at,
                auth_method: entry.auth_method,
                has_profile_arn: entry.has_profile_arn,
                refresh_token_hash: entry.refresh_token_hash,
                email: entry.email,
                success_count: entry.success_count,
                last_used_at: entry.last_used_at.clone(),
            })
            .collect();

        // Sort by priority (lower number = higher priority)
        credentials.sort_by_key(|c| c.priority);

        CredentialsStatusResponse {
            total: snapshot.total,
            available: snapshot.available,
            current_id: snapshot.current_id,
            credentials,
        }
    }

    /// Set credential disabled status
    pub fn set_disabled(&self, id: u64, disabled: bool) -> Result<(), AdminServiceError> {
        // First get current credential ID to determine if switch is needed
        let snapshot = self.token_manager.snapshot();
        let current_id = snapshot.current_id;

        self.token_manager
            .set_disabled(id, disabled)
            .map_err(|e| self.classify_error(e, id))?;

        // Only try to switch to next when disabling the current credential
        if disabled && id == current_id {
            let _ = self.token_manager.switch_to_next();
        }
        Ok(())
    }

    /// Set credential priority
    pub fn set_priority(&self, id: u64, priority: u32) -> Result<(), AdminServiceError> {
        self.token_manager
            .set_priority(id, priority)
            .map_err(|e| self.classify_error(e, id))
    }

    /// Reset failure count and re-enable
    pub fn reset_and_enable(&self, id: u64) -> Result<(), AdminServiceError> {
        self.token_manager
            .reset_and_enable(id)
            .map_err(|e| self.classify_error(e, id))
    }

    /// Get credential balance (with cache)
    pub async fn get_balance(&self, id: u64) -> Result<BalanceResponse, AdminServiceError> {
        // Check cache first
        {
            let cache = self.balance_cache.lock();
            if let Some(cached) = cache.get(&id) {
                let now = Utc::now().timestamp() as f64;
                if (now - cached.cached_at) < BALANCE_CACHE_TTL_SECS as f64 {
                    tracing::debug!("Credential #{} balance cache hit", id);
                    return Ok(cached.data.clone());
                }
            }
        }

        // Cache miss or expired, fetch from upstream
        let balance = self.fetch_balance(id).await?;

        // Update cache
        {
            let mut cache = self.balance_cache.lock();
            cache.insert(
                id,
                CachedBalance {
                    cached_at: Utc::now().timestamp() as f64,
                    data: balance.clone(),
                },
            );
        }
        self.save_balance_cache();

        Ok(balance)
    }

    /// Fetch balance from upstream (no cache)
    async fn fetch_balance(&self, id: u64) -> Result<BalanceResponse, AdminServiceError> {
        let usage = self
            .token_manager
            .get_usage_limits_for(id)
            .await
            .map_err(|e| self.classify_balance_error(e, id))?;

        let current_usage = usage.current_usage();
        let usage_limit = usage.usage_limit();
        let remaining = (usage_limit - current_usage).max(0.0);
        let usage_percentage = if usage_limit > 0.0 {
            (current_usage / usage_limit * 100.0).min(100.0)
        } else {
            0.0
        };

        Ok(BalanceResponse {
            id,
            email: usage.email().map(|s| s.to_string()),
            subscription_title: usage.subscription_title().map(|s| s.to_string()),
            current_usage,
            usage_limit,
            remaining,
            usage_percentage,
            next_reset_at: usage.next_date_reset,
        })
    }

    /// Add new credential
    pub async fn add_credential(
        &self,
        req: AddCredentialRequest,
    ) -> Result<AddCredentialResponse, AdminServiceError> {
        // Build credential object
        let email = req.email.clone();
        let new_cred = KiroCredentials {
            id: None,
            access_token: None,
            refresh_token: Some(req.refresh_token),
            profile_arn: None,
            expires_at: None,
            auth_method: Some(req.auth_method),
            client_id: req.client_id,
            client_secret: req.client_secret,
            priority: req.priority,
            region: req.region,
            auth_region: req.auth_region,
            api_region: req.api_region,
            machine_id: req.machine_id,
            email: req.email,
            subscription_title: None,
        };

        // Call token_manager to add credential
        let credential_id = self
            .token_manager
            .add_credential(new_cred)
            .await
            .map_err(|e| self.classify_add_error(e))?;

        Ok(AddCredentialResponse {
            success: true,
            message: format!("Credential added successfully, ID: {}", credential_id),
            credential_id,
            email,
        })
    }

    /// Delete credential
    pub fn delete_credential(&self, id: u64) -> Result<(), AdminServiceError> {
        self.token_manager
            .delete_credential(id)
            .map_err(|e| self.classify_delete_error(e, id))?;

        // Clean up balance cache for deleted credential
        {
            let mut cache = self.balance_cache.lock();
            cache.remove(&id);
        }
        self.save_balance_cache();

        Ok(())
    }

    /// Get load balancing mode
    pub fn get_load_balancing_mode(&self) -> LoadBalancingModeResponse {
        LoadBalancingModeResponse {
            mode: self.token_manager.get_load_balancing_mode(),
        }
    }

    /// Set load balancing mode
    pub fn set_load_balancing_mode(
        &self,
        req: SetLoadBalancingModeRequest,
    ) -> Result<LoadBalancingModeResponse, AdminServiceError> {
        // Validate mode value
        if req.mode != "priority" && req.mode != "balanced" {
            return Err(AdminServiceError::InvalidCredential(
                "mode must be 'priority' or 'balanced'".to_string(),
            ));
        }

        self.token_manager
            .set_load_balancing_mode(req.mode.clone())
            .map_err(|e| AdminServiceError::InternalError(e.to_string()))?;

        Ok(LoadBalancingModeResponse { mode: req.mode })
    }

    /// Force refresh token for a credential
    pub async fn refresh_token(&self, id: u64) -> Result<(), AdminServiceError> {
        self.token_manager
            .force_refresh_token(id)
            .await
            .map_err(|e| self.classify_refresh_error(e, id))
    }

    /// Classify refresh errors
    fn classify_refresh_error(&self, error: anyhow::Error, id: u64) -> AdminServiceError {
        let msg = error.to_string();

        if msg.contains("does not exist") {
            return AdminServiceError::CredentialNotFound(id);
        }

        if msg.contains("expired") || msg.contains("invalid") || msg.contains("401") {
            return AdminServiceError::TokenRefreshFailed(format!(
                "Token refresh failed for credential #{}: {}",
                id, msg
            ));
        }

        AdminServiceError::InternalError(format!("Refresh failed: {}", msg))
    }

    // ============ Balance cache persistence ============

    fn load_balance_cache_from(cache_path: &Option<PathBuf>) -> HashMap<u64, CachedBalance> {
        let path = match cache_path {
            Some(p) => p,
            None => return HashMap::new(),
        };

        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return HashMap::new(),
        };

        // File uses string keys for JSON format compatibility
        let map: HashMap<String, CachedBalance> = match serde_json::from_str(&content) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Failed to parse balance cache, ignoring: {}", e);
                return HashMap::new();
            }
        };

        let now = Utc::now().timestamp() as f64;
        map.into_iter()
            .filter_map(|(k, v)| {
                let id = k.parse::<u64>().ok()?;
                // Discard entries exceeding TTL
                if (now - v.cached_at) < BALANCE_CACHE_TTL_SECS as f64 {
                    Some((id, v))
                } else {
                    None
                }
            })
            .collect()
    }

    fn save_balance_cache(&self) {
        let path = match &self.cache_path {
            Some(p) => p,
            None => return,
        };

        // Hold lock during serialization and write to prevent concurrent corruption
        let cache = self.balance_cache.lock();
        let map: HashMap<String, &CachedBalance> =
            cache.iter().map(|(k, v)| (k.to_string(), v)).collect();

        match serde_json::to_string_pretty(&map) {
            Ok(json) => {
                if let Err(e) = std::fs::write(path, json) {
                    tracing::warn!("Failed to save balance cache: {}", e);
                }
            }
            Err(e) => tracing::warn!("Failed to serialize balance cache: {}", e),
        }
    }

    // ============ Error classification ============

    /// Classify simple operation errors (set_disabled, set_priority, reset_and_enable)
    fn classify_error(&self, e: anyhow::Error, id: u64) -> AdminServiceError {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("does not exist") {
            AdminServiceError::NotFound { id }
        } else {
            AdminServiceError::InternalError(msg)
        }
    }

    /// Classify balance query errors (may involve upstream API calls)
    fn classify_balance_error(&self, e: anyhow::Error, id: u64) -> AdminServiceError {
        let msg = e.to_string();

        // 1. Credential not found
        if msg.contains("not found") || msg.contains("does not exist") {
            return AdminServiceError::NotFound { id };
        }

        // 2. Upstream service error characteristics: HTTP response errors or network errors
        let is_upstream_error =
            // HTTP response errors (from refresh_*_token error messages)
            msg.contains("credential expired or invalid") ||
            msg.contains("insufficient permissions") ||
            msg.contains("rate limited") ||
            msg.contains("server error") ||
            msg.contains("Token refresh failed") ||
            msg.contains("temporarily unavailable") ||
            // Network errors (reqwest errors)
            msg.contains("error trying to connect") ||
            msg.contains("connection") ||
            msg.contains("timeout") ||
            msg.contains("timed out");

        if is_upstream_error {
            AdminServiceError::UpstreamError(msg)
        } else {
            // 3. Default to internal error (local validation failure, configuration error, etc.)
            // Includes: missing refreshToken, truncated refreshToken, unable to generate machineId, etc.
            AdminServiceError::InternalError(msg)
        }
    }

    /// Classify add credential errors
    fn classify_add_error(&self, e: anyhow::Error) -> AdminServiceError {
        let msg = e.to_string();

        // Credential validation failure (invalid refreshToken, format error, etc.)
        let is_invalid_credential = msg.contains("missing refreshToken")
            || msg.contains("refreshToken is empty")
            || msg.contains("refreshToken has been truncated")
            || msg.contains("credential already exists")
            || msg.contains("duplicate refreshToken")
            || msg.contains("credential expired or invalid")
            || msg.contains("insufficient permissions")
            || msg.contains("rate limited");

        if is_invalid_credential {
            AdminServiceError::InvalidCredential(msg)
        } else if msg.contains("error trying to connect")
            || msg.contains("connection")
            || msg.contains("timeout")
        {
            AdminServiceError::UpstreamError(msg)
        } else {
            AdminServiceError::InternalError(msg)
        }
    }

    /// Classify delete credential errors
    fn classify_delete_error(&self, e: anyhow::Error, id: u64) -> AdminServiceError {
        let msg = e.to_string();
        if msg.contains("not found") || msg.contains("does not exist") {
            AdminServiceError::NotFound { id }
        } else if msg.contains("can only delete disabled credentials") || msg.contains("please disable the credential first") {
            AdminServiceError::InvalidCredential(msg)
        } else {
            AdminServiceError::InternalError(msg)
        }
    }
}
