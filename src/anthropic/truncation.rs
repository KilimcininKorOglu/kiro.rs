//! Tool call truncation detection module
//!
//! When Kiro API reaches output token limit, tool call JSON may be truncated,
//! resulting in incomplete parameters or unparseable data. This module detects
//! truncation and generates soft failure messages to guide retry.

use std::collections::{HashMap, HashSet};

/// Truncation type
#[derive(Debug, Clone, PartialEq)]
pub enum TruncationType {
    /// No truncation
    None,
    /// Input completely empty
    EmptyInput,
    /// Invalid JSON syntax (truncated mid-value)
    InvalidJson,
    /// JSON parsed but missing critical fields
    MissingFields,
    /// String value was truncated
    IncompleteString,
}

/// Truncation detection info
#[derive(Debug, Clone)]
pub struct TruncationInfo {
    pub is_truncated: bool,
    pub truncation_type: TruncationType,
    pub tool_name: String,
    pub tool_use_id: String,
    pub raw_input: String,
    pub parsed_fields: HashMap<String, String>,
    pub error_message: String,
}

/// Known write tools
fn is_write_tool(name: &str) -> bool {
    matches!(
        name,
        "Write"
            | "write_to_file"
            | "fsWrite"
            | "create_file"
            | "edit_file"
            | "apply_diff"
            | "str_replace_editor"
            | "insert"
            | "Create"
            | "Edit"
            | "MultiEdit"
    )
}

/// Required fields mapping for tools
fn required_fields(tool_name: &str) -> Option<&[&str]> {
    match tool_name {
        "Write" | "Create" => Some(&["file_path", "content"]),
        "write_to_file" | "fsWrite" | "create_file" => Some(&["path", "content"]),
        "edit_file" | "Edit" => Some(&["file_path", "old_str", "new_str"]),
        "apply_diff" => Some(&["path", "diff"]),
        "str_replace_editor" => Some(&["path", "old_str", "new_str"]),
        "Bash" | "Execute" | "execute" | "run_command" => Some(&["command"]),
        "Read" => Some(&["file_path"]),
        "Grep" => Some(&["pattern"]),
        "Glob" => Some(&["patterns"]),
        _ => None,
    }
}

/// Detect if tool input was truncated
pub fn detect_truncation(
    tool_name: &str,
    tool_use_id: &str,
    raw_input: &str,
    parsed_input: Option<&serde_json::Value>,
) -> TruncationInfo {
    let mut info = TruncationInfo {
        is_truncated: false,
        truncation_type: TruncationType::None,
        tool_name: tool_name.to_string(),
        tool_use_id: tool_use_id.to_string(),
        raw_input: raw_input.to_string(),
        parsed_fields: HashMap::new(),
        error_message: String::new(),
    };

    // Scenario 1: Input completely empty
    if raw_input.trim().is_empty() {
        info.is_truncated = true;
        info.truncation_type = TruncationType::EmptyInput;
        info.error_message =
            "Tool input was completely empty - API response may have been truncated".to_string();
        tracing::warn!(
            "Truncation detected [empty_input] tool={} id={}: input is empty",
            tool_name,
            tool_use_id
        );
        return info;
    }

    // Scenario 2: JSON parse failed
    let parsed = match parsed_input {
        Some(v) if v.is_object() && !v.as_object().unwrap().is_empty() => Some(v),
        _ => None,
    };

    if parsed.is_none() && looks_like_truncated_json(raw_input) {
        info.is_truncated = true;
        info.truncation_type = TruncationType::InvalidJson;
        info.parsed_fields = extract_partial_fields(raw_input);
        info.error_message = format!(
            "Tool input JSON was truncated mid-transmission ({} bytes received)",
            raw_input.len()
        );
        tracing::warn!(
            "Truncation detected [invalid_json] tool={} id={}: JSON parse failed, raw_len={}",
            tool_name,
            tool_use_id,
            raw_input.len()
        );
        return info;
    }

    // Scenario 3: JSON parsed but missing required fields
    if let Some(parsed_val) = parsed {
        if let Some(obj) = parsed_val.as_object() {
            if let Some(required) = required_fields(tool_name) {
                let existing: HashSet<&str> = obj.keys().map(|k| k.as_str()).collect();
                let missing: Vec<&&str> = required
                    .iter()
                    .filter(|f| !existing.contains(**f))
                    .collect();

                if !missing.is_empty() {
                    info.is_truncated = true;
                    info.truncation_type = TruncationType::MissingFields;
                    info.parsed_fields = extract_parsed_field_names(obj);
                    info.error_message = format!(
                        "Tool '{}' missing required fields: {}",
                        tool_name,
                        missing.iter().map(|f| **f).collect::<Vec<_>>().join(", ")
                    );
                    tracing::warn!(
                        "Truncation detected [missing_fields] tool={} id={}: missing {:?}",
                        tool_name,
                        tool_use_id,
                        missing
                    );
                    return info;
                }
            }

            // Scenario 4: Write tool content field truncated
            if is_write_tool(tool_name) {
                if let Some(msg) = detect_content_truncation(obj, raw_input) {
                    info.is_truncated = true;
                    info.truncation_type = TruncationType::IncompleteString;
                    info.parsed_fields = extract_parsed_field_names(obj);
                    info.error_message = msg;
                    tracing::warn!(
                        "Truncation detected [incomplete_string] tool={} id={}: {}",
                        tool_name,
                        tool_use_id,
                        info.error_message
                    );
                    return info;
                }
            }
        }
    }

    info
}

/// Check if raw string looks like truncated JSON
fn looks_like_truncated_json(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.starts_with('{') {
        return false;
    }

    // Unbalanced brackets
    let open_braces = trimmed.matches('{').count();
    let close_braces = trimmed.matches('}').count();
    let open_brackets = trimmed.matches('[').count();
    let close_brackets = trimmed.matches(']').count();

    if open_braces > close_braces || open_brackets > close_brackets {
        return true;
    }

    // Abnormal trailing character
    if let Some(last) = trimmed.bytes().last() {
        if last != b'}' && last != b']' && (last == b'"' || last == b':' || last == b',') {
            return true;
        }
    }

    // Unclosed string (odd number of unescaped quotes)
    let mut in_string = false;
    let mut escaped = false;
    for b in trimmed.bytes() {
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' {
            escaped = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
        }
    }
    if in_string {
        return true;
    }

    false
}

/// Extract partial field names from malformed JSON
fn extract_partial_fields(raw: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let trimmed = raw.trim().strip_prefix('{').unwrap_or(raw);

    for part in trimmed.split(',') {
        let part = part.trim();
        if let Some(colon_idx) = part.find(':') {
            let key = part[..colon_idx].trim().trim_matches('"');
            let value = part[colon_idx + 1..].trim();
            let display_value = if value.len() > 50 {
                value.chars().take(50).collect::<String>() + "..."
            } else {
                value.to_string()
            };
            fields.insert(key.to_string(), display_value);
        }
    }

    fields
}

/// Extract field names from parsed JSON object
fn extract_parsed_field_names(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for (key, val) in obj {
        let display = match val {
            serde_json::Value::String(s) => {
                if s.len() > 50 {
                    format!("{}...", &s.chars().take(50).collect::<String>())
                } else {
                    s.clone()
                }
            }
            serde_json::Value::Null => "<null>".to_string(),
            _ => "<present>".to_string(),
        };
        fields.insert(key.clone(), display);
    }
    fields
}

/// Detect if write tool content field was truncated
fn detect_content_truncation(
    obj: &serde_json::Map<String, serde_json::Value>,
    raw_input: &str,
) -> Option<String> {
    let content = obj.get("content")?.as_str()?;

    // Heuristic: raw input is large but content field is suspiciously short
    if raw_input.len() > 1000 && content.len() < 100 {
        return Some(
            "content field appears suspiciously short compared to raw input size".to_string(),
        );
    }

    // Check for unclosed code fences
    if content.contains("```") {
        let fence_count = content.matches("```").count();
        if fence_count % 2 != 0 {
            return Some(
                "content contains unclosed code fence (```) suggesting truncation".to_string(),
            );
        }
    }

    None
}

/// Build soft failure tool result message
///
/// When truncation is detected, return this message as tool_result to guide Claude to retry
pub fn build_soft_failure_result(info: &TruncationInfo) -> String {
    let max_line_hint = match info.truncation_type {
        TruncationType::EmptyInput => 200,
        TruncationType::InvalidJson => 250,
        TruncationType::MissingFields => 300,
        TruncationType::IncompleteString => 350,
        TruncationType::None => 300,
    };

    let reason = match info.truncation_type {
        TruncationType::EmptyInput => {
            "Your tool call was too large and the input was completely lost during transmission."
        }
        TruncationType::InvalidJson => {
            "Your tool call was truncated mid-transmission, resulting in incomplete JSON."
        }
        TruncationType::MissingFields => {
            "Your tool call was partially received but critical fields were cut off."
        }
        TruncationType::IncompleteString => {
            "Your tool call content was truncated - the full content did not arrive."
        }
        TruncationType::None => {
            "Your tool call was truncated by the API due to output size limits."
        }
    };

    let mut result = format!(
        "TOOL_CALL_INCOMPLETE\nstatus: incomplete\nreason: {}\n",
        reason
    );

    if !info.parsed_fields.is_empty() {
        let fields: Vec<String> = info
            .parsed_fields
            .iter()
            .map(|(k, v)| {
                let display_v: String = if v.len() > 30 {
                    v.chars().take(30).collect::<String>() + "..."
                } else {
                    v.clone()
                };
                format!("{}={}", k, display_v)
            })
            .collect();
        result.push_str(&format!(
            "context: Received partial data: {}\n",
            fields.join(", ")
        ));
    }

    result.push_str(&format!(
        "\nCONCLUSION: Split your output into smaller chunks and retry.\n\
         \n\
         REQUIRED APPROACH:\n\
         1. For file writes: Write in chunks of ~{} lines maximum\n\
         2. For new files: First create with initial chunk, then append remaining sections\n\
         3. For edits: Make surgical, targeted changes - avoid rewriting entire files\n\
         \n\
         DO NOT attempt to write the full content again in a single call.\n\
         The API has a hard output limit that cannot be bypassed.\n",
        max_line_hint
    ));

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_empty_input() {
        let info = detect_truncation("Write", "test-id", "", None);
        assert!(info.is_truncated);
        assert_eq!(info.truncation_type, TruncationType::EmptyInput);
    }

    #[test]
    fn test_detect_truncated_json() {
        let raw = r#"{"file_path": "/test.txt", "content": "hello"#;
        let info = detect_truncation("Write", "test-id", raw, None);
        assert!(info.is_truncated);
        assert_eq!(info.truncation_type, TruncationType::InvalidJson);
    }

    #[test]
    fn test_detect_missing_fields() {
        let raw = r#"{"file_path": "/test.txt"}"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let info = detect_truncation("Write", "test-id", raw, Some(&parsed));
        assert!(info.is_truncated);
        assert_eq!(info.truncation_type, TruncationType::MissingFields);
    }

    #[test]
    fn test_valid_input_no_truncation() {
        let raw = r#"{"file_path": "/test.txt", "content": "hello world"}"#;
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap();
        let info = detect_truncation("Write", "test-id", raw, Some(&parsed));
        assert!(!info.is_truncated);
        assert_eq!(info.truncation_type, TruncationType::None);
    }

    #[test]
    fn test_looks_like_truncated_json() {
        assert!(looks_like_truncated_json(r#"{"key": "value"#));
        assert!(looks_like_truncated_json(r#"{"key": "#));
        assert!(!looks_like_truncated_json(r#"{"key": "value"}"#));
    }

    #[test]
    fn test_is_write_tool() {
        assert!(is_write_tool("Write"));
        assert!(is_write_tool("Create"));
        assert!(is_write_tool("Edit"));
        assert!(!is_write_tool("Read"));
        assert!(!is_write_tool("Grep"));
    }

    #[test]
    fn test_build_soft_failure_result() {
        let info = TruncationInfo {
            is_truncated: true,
            truncation_type: TruncationType::InvalidJson,
            tool_name: "Write".to_string(),
            tool_use_id: "test-id".to_string(),
            raw_input: "{}".to_string(),
            parsed_fields: HashMap::new(),
            error_message: "Test error".to_string(),
        };
        let result = build_soft_failure_result(&info);
        assert!(result.contains("TOOL_CALL_INCOMPLETE"));
        assert!(result.contains("truncated mid-transmission"));
    }
}
