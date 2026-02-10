//! Device fingerprint generator
//!

use sha2::{Digest, Sha256};

use crate::kiro::model::credentials::KiroCredentials;
use crate::model::config::Config;

/// Normalize machineId format
///
/// Supports the following formats:
/// - 64-character hexadecimal string (returned as-is)
/// - UUID format (e.g., "2582956e-cc88-4669-b546-07adbffcb894", removes dashes and pads to 64 characters)
fn normalize_machine_id(machine_id: &str) -> Option<String> {
    let trimmed = machine_id.trim();

    // If already 64 characters, return directly
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(trimmed.to_string());
    }

    // Try to parse UUID format (remove dashes)
    let without_dashes: String = trimmed.chars().filter(|c| *c != '-').collect();

    // UUID without dashes is 32 characters
    if without_dashes.len() == 32 && without_dashes.chars().all(|c| c.is_ascii_hexdigit()) {
        // Pad to 64 characters (repeat once)
        return Some(format!("{}{}", without_dashes, without_dashes));
    }

    // Unrecognized format
    None
}

/// Generate unique Machine ID based on credential information
///
/// Priority: credential-level machineId > config.machineId > generated from refreshToken
pub fn generate_from_credentials(credentials: &KiroCredentials, config: &Config) -> Option<String> {
    // If credential-level machineId is configured, use it first
    if let Some(ref machine_id) = credentials.machine_id {
        if let Some(normalized) = normalize_machine_id(machine_id) {
            return Some(normalized);
        }
    }

    // If global machineId is configured, use as default
    if let Some(ref machine_id) = config.machine_id {
        if let Some(normalized) = normalize_machine_id(machine_id) {
            return Some(normalized);
        }
    }

    // Generate from refreshToken
    if let Some(ref refresh_token) = credentials.refresh_token {
        if !refresh_token.is_empty() {
            return Some(sha256_hex(&format!("KotlinNativeAPI/{}", refresh_token)));
        }
    }

    // No valid credentials
    None
}

/// SHA256 hash implementation (returns hexadecimal string)
fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let result = hasher.finalize();
    hex::encode(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hex() {
        let result = sha256_hex("test");
        assert_eq!(result.len(), 64);
        assert_eq!(
            result,
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        );
    }

    #[test]
    fn test_generate_with_custom_machine_id() {
        let credentials = KiroCredentials::default();
        let mut config = Config::default();
        config.machine_id = Some("a".repeat(64));

        let result = generate_from_credentials(&credentials, &config);
        assert_eq!(result, Some("a".repeat(64)));
    }

    #[test]
    fn test_generate_with_credential_machine_id_overrides_config() {
        let mut credentials = KiroCredentials::default();
        credentials.machine_id = Some("b".repeat(64));

        let mut config = Config::default();
        config.machine_id = Some("a".repeat(64));

        let result = generate_from_credentials(&credentials, &config);
        assert_eq!(result, Some("b".repeat(64)));
    }

    #[test]
    fn test_generate_with_refresh_token() {
        let mut credentials = KiroCredentials::default();
        credentials.refresh_token = Some("test_refresh_token".to_string());
        let config = Config::default();

        let result = generate_from_credentials(&credentials, &config);
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().len(), 64);
    }

    #[test]
    fn test_generate_without_credentials() {
        let credentials = KiroCredentials::default();
        let config = Config::default();

        let result = generate_from_credentials(&credentials, &config);
        assert!(result.is_none());
    }

    #[test]
    fn test_normalize_uuid_format() {
        // UUID format should be converted to 64 characters
        let uuid = "2582956e-cc88-4669-b546-07adbffcb894";
        let result = normalize_machine_id(uuid);
        assert!(result.is_some());
        let normalized = result.unwrap();
        assert_eq!(normalized.len(), 64);
        // UUID without dashes repeated once
        assert_eq!(
            normalized,
            "2582956ecc884669b54607adbffcb8942582956ecc884669b54607adbffcb894"
        );
    }

    #[test]
    fn test_normalize_64_char_hex() {
        // 64-character hex should be returned directly
        let hex64 = "a".repeat(64);
        let result = normalize_machine_id(&hex64);
        assert_eq!(result, Some(hex64));
    }

    #[test]
    fn test_normalize_invalid_format() {
        // Invalid format should return None
        assert!(normalize_machine_id("invalid").is_none());
        assert!(normalize_machine_id("too-short").is_none());
        assert!(normalize_machine_id(&"g".repeat(64)).is_none()); // Non-hexadecimal
    }

    #[test]
    fn test_generate_with_uuid_machine_id() {
        let mut credentials = KiroCredentials::default();
        credentials.machine_id = Some("2582956e-cc88-4669-b546-07adbffcb894".to_string());

        let config = Config::default();

        let result = generate_from_credentials(&credentials, &config);
        assert!(result.is_some());
        assert_eq!(result.as_ref().unwrap().len(), 64);
    }
}
