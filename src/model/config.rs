use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TlsBackend {
    Rustls,
    NativeTls,
}

impl Default for TlsBackend {
    fn default() -> Self {
        Self::Rustls
    }
}

/// KNA application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default = "default_host")]
    pub host: String,

    #[serde(default = "default_port")]
    pub port: u16,

    #[serde(default = "default_region")]
    pub region: String,

    /// Auth Region (for token refresh), falls back to region if not configured
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_region: Option<String>,

    /// API Region (for API requests), falls back to region if not configured
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_region: Option<String>,

    #[serde(default = "default_kiro_version")]
    pub kiro_version: String,

    #[serde(default)]
    pub machine_id: Option<String>,

    #[serde(default)]
    pub api_key: Option<String>,

    #[serde(default = "default_system_version")]
    pub system_version: String,

    #[serde(default = "default_node_version")]
    pub node_version: String,

    #[serde(default = "default_tls_backend")]
    pub tls_backend: TlsBackend,

    /// External count_tokens API URL (optional)
    #[serde(default)]
    pub count_tokens_api_url: Option<String>,

    /// count_tokens API key (optional)
    #[serde(default)]
    pub count_tokens_api_key: Option<String>,

    /// count_tokens API auth type (optional, "x-api-key" or "bearer", default "x-api-key")
    #[serde(default = "default_count_tokens_auth_type")]
    pub count_tokens_auth_type: String,

    /// HTTP proxy URL (optional)
    /// Supported formats: http://host:port, https://host:port, socks5://host:port
    #[serde(default)]
    pub proxy_url: Option<String>,

    /// Proxy authentication username (optional)
    #[serde(default)]
    pub proxy_username: Option<String>,

    /// Proxy authentication password (optional)
    #[serde(default)]
    pub proxy_password: Option<String>,

    /// Admin API key (optional, enables Admin API functionality)
    #[serde(default)]
    pub admin_api_key: Option<String>,

    /// Load balancing mode ("priority" or "balanced")
    #[serde(default = "default_load_balancing_mode")]
    pub load_balancing_mode: String,

    /// Model name suffix to trigger thinking mode (default: "-thinking")
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_suffix: Option<String>,

    /// Thinking output format: "thinking", "think", or "reasoning_content"
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_format: Option<String>,

    /// Maximum request body size in bytes (0 = unlimited, default: 400000)
    #[serde(default = "default_max_request_body_bytes")]
    pub max_request_body_bytes: usize,

    /// Config file path (runtime metadata, not written to JSON)
    #[serde(skip)]
    config_path: Option<PathBuf>,
}

fn default_host() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_region() -> String {
    "us-east-1".to_string()
}

fn default_kiro_version() -> String {
    "0.9.2".to_string()
}

fn default_system_version() -> String {
    const SYSTEM_VERSIONS: &[&str] = &["darwin#24.6.0", "win32#10.0.22631"];
    SYSTEM_VERSIONS[fastrand::usize(..SYSTEM_VERSIONS.len())].to_string()
}

fn default_node_version() -> String {
    "22.21.1".to_string()
}

fn default_count_tokens_auth_type() -> String {
    "x-api-key".to_string()
}

fn default_tls_backend() -> TlsBackend {
    TlsBackend::Rustls
}

fn default_load_balancing_mode() -> String {
    "priority".to_string()
}

fn default_max_request_body_bytes() -> usize {
    400_000
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            region: default_region(),
            auth_region: None,
            api_region: None,
            kiro_version: default_kiro_version(),
            machine_id: None,
            api_key: None,
            system_version: default_system_version(),
            node_version: default_node_version(),
            tls_backend: default_tls_backend(),
            count_tokens_api_url: None,
            count_tokens_api_key: None,
            count_tokens_auth_type: default_count_tokens_auth_type(),
            proxy_url: None,
            proxy_username: None,
            proxy_password: None,
            admin_api_key: None,
            load_balancing_mode: default_load_balancing_mode(),
            thinking_suffix: None,
            thinking_format: None,
            max_request_body_bytes: default_max_request_body_bytes(),
            config_path: None,
        }
    }
}

impl Config {
    /// Get default config file path
    pub fn default_config_path() -> &'static str {
        "config.json"
    }

    /// Get thinking suffix (default: "-thinking")
    pub fn thinking_suffix(&self) -> &str {
        self.thinking_suffix.as_deref().unwrap_or("-thinking")
    }

    /// Get thinking format (default: "thinking")
    pub fn thinking_format(&self) -> &str {
        self.thinking_format.as_deref().unwrap_or("thinking")
    }

    /// Get effective Auth Region (for token refresh)
    /// Prefers auth_region, falls back to region if not configured
    pub fn effective_auth_region(&self) -> &str {
        self.auth_region.as_deref().unwrap_or(&self.region)
    }

    /// Get effective API Region (for API requests)
    /// Prefers api_region, falls back to region if not configured
    pub fn effective_api_region(&self) -> &str {
        self.api_region.as_deref().unwrap_or(&self.region)
    }

    /// Load configuration from file
    pub fn load<P: AsRef<Path>>(path: P) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            // Config file doesn't exist, return default config
            let mut config = Self::default();
            config.config_path = Some(path.to_path_buf());
            return Ok(config);
        }

        let content = fs::read_to_string(path)?;
        let mut config: Config = serde_json::from_str(&content)?;
        config.config_path = Some(path.to_path_buf());
        Ok(config)
    }

    /// Get config file path (if available)
    pub fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// Write current config back to original config file
    pub fn save(&self) -> anyhow::Result<()> {
        let path = self
            .config_path
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Config file path unknown, cannot save config"))?;

        let content = serde_json::to_string_pretty(self).context("Failed to serialize config")?;
        fs::write(path, content).with_context(|| format!("Failed to write config file: {}", path.display()))?;
        Ok(())
    }
}
