//! Kiro API error enhancement module
//!
//! Transforms cryptic Kiro API errors into user-friendly messages.

use serde_json::Value;

/// Structured information about a Kiro API error
#[derive(Debug, Clone)]
pub struct KiroErrorInfo {
    /// Error reason code from Kiro API
    pub reason: String,
    /// Enhanced, user-friendly message for end users
    pub user_message: String,
    /// Original message from Kiro API (for logging)
    pub original_message: String,
}

/// Enhances Kiro API error with user-friendly message
///
/// Takes raw error JSON from Kiro API and returns structured information
/// with enhanced, user-friendly messages.
///
/// # Arguments
/// * `error_json` - Parsed JSON from Kiro API error response
///                  Expected format: {"message": "...", "reason": "..."}
///
/// # Returns
/// KiroErrorInfo with enhanced message and original details
pub fn enhance_kiro_error(error_json: &Value) -> KiroErrorInfo {
    let original_message = error_json
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown error")
        .to_string();

    let reason = error_json
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN")
        .to_string();

    let user_message = match reason.as_str() {
        "CONTENT_LENGTH_EXCEEDS_THRESHOLD" => {
            "Model context limit reached. Conversation size exceeds model capacity.".to_string()
        }
        "MONTHLY_REQUEST_LIMIT_REACHED" | "MONTHLY_REQUEST_COUNT" => {
            "Monthly request limit exceeded. Account has reached its monthly quota.".to_string()
        }
        "RATE_LIMIT_EXCEEDED" => {
            "Rate limit exceeded. Please wait a moment before retrying.".to_string()
        }
        "SERVICE_UNAVAILABLE" => {
            "Kiro service temporarily unavailable. Please try again later.".to_string()
        }
        "THROTTLING_EXCEPTION" => {
            "Too many requests. Please slow down and try again.".to_string()
        }
        "VALIDATION_EXCEPTION" => {
            format!("Invalid request: {}", original_message)
        }
        "UNKNOWN" => original_message.clone(),
        _ => {
            // Unknown error - keep original message with reason suffix
            format!("{} (reason: {})", original_message, reason)
        }
    };

    KiroErrorInfo {
        reason,
        user_message,
        original_message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_content_length_error_enhanced() {
        let error_json = json!({
            "message": "Input is too long.",
            "reason": "CONTENT_LENGTH_EXCEEDS_THRESHOLD"
        });

        let error_info = enhance_kiro_error(&error_json);

        assert_eq!(
            error_info.user_message,
            "Model context limit reached. Conversation size exceeds model capacity."
        );
        assert_eq!(error_info.reason, "CONTENT_LENGTH_EXCEEDS_THRESHOLD");
        assert_eq!(error_info.original_message, "Input is too long.");
    }

    #[test]
    fn test_monthly_limit_error_enhanced() {
        let error_json = json!({
            "message": "Monthly request limit exceeded.",
            "reason": "MONTHLY_REQUEST_LIMIT_REACHED"
        });

        let error_info = enhance_kiro_error(&error_json);

        assert!(error_info.user_message.contains("Monthly request limit"));
    }

    #[test]
    fn test_monthly_request_count_error_enhanced() {
        let error_json = json!({
            "message": "You have reached the limit.",
            "reason": "MONTHLY_REQUEST_COUNT"
        });

        let error_info = enhance_kiro_error(&error_json);

        assert_eq!(
            error_info.user_message,
            "Monthly request limit exceeded. Account has reached its monthly quota."
        );
        assert_eq!(error_info.reason, "MONTHLY_REQUEST_COUNT");
    }

    #[test]
    fn test_rate_limit_error_enhanced() {
        let error_json = json!({
            "message": "Too many requests.",
            "reason": "RATE_LIMIT_EXCEEDED"
        });

        let error_info = enhance_kiro_error(&error_json);

        assert!(error_info.user_message.contains("Rate limit exceeded"));
    }

    #[test]
    fn test_unknown_reason_keeps_original_with_suffix() {
        let error_json = json!({
            "message": "Something went wrong.",
            "reason": "UNKNOWN_FUTURE_ERROR"
        });

        let error_info = enhance_kiro_error(&error_json);

        assert_eq!(
            error_info.user_message,
            "Something went wrong. (reason: UNKNOWN_FUTURE_ERROR)"
        );
        assert_eq!(error_info.reason, "UNKNOWN_FUTURE_ERROR");
        assert_eq!(error_info.original_message, "Something went wrong.");
    }

    #[test]
    fn test_missing_reason_uses_unknown() {
        let error_json = json!({
            "message": "An error occurred."
        });

        let error_info = enhance_kiro_error(&error_json);

        assert_eq!(error_info.reason, "UNKNOWN");
        assert_eq!(error_info.user_message, "An error occurred.");
    }

    #[test]
    fn test_missing_message_uses_default() {
        let error_json = json!({
            "reason": "CONTENT_LENGTH_EXCEEDS_THRESHOLD"
        });

        let error_info = enhance_kiro_error(&error_json);

        assert_eq!(error_info.original_message, "Unknown error");
        assert!(error_info.user_message.contains("context limit"));
    }

    #[test]
    fn test_empty_json_uses_defaults() {
        let error_json = json!({});

        let error_info = enhance_kiro_error(&error_json);

        assert_eq!(error_info.original_message, "Unknown error");
        assert_eq!(error_info.reason, "UNKNOWN");
        assert_eq!(error_info.user_message, "Unknown error");
    }

    #[test]
    fn test_throttling_exception_enhanced() {
        let error_json = json!({
            "message": "Rate exceeded.",
            "reason": "THROTTLING_EXCEPTION"
        });

        let error_info = enhance_kiro_error(&error_json);

        assert!(error_info.user_message.contains("Too many requests"));
    }

    #[test]
    fn test_validation_exception_includes_original() {
        let error_json = json!({
            "message": "Invalid model ID.",
            "reason": "VALIDATION_EXCEPTION"
        });

        let error_info = enhance_kiro_error(&error_json);

        assert!(error_info.user_message.contains("Invalid request"));
        assert!(error_info.user_message.contains("Invalid model ID"));
    }

    #[test]
    fn test_service_unavailable_enhanced() {
        let error_json = json!({
            "message": "Service is down.",
            "reason": "SERVICE_UNAVAILABLE"
        });

        let error_info = enhance_kiro_error(&error_json);

        assert!(error_info.user_message.contains("temporarily unavailable"));
    }
}
