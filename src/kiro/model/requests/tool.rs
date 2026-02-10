//! Tool type definitions
//!
//! Defines tool-related types for Kiro API

use serde::{Deserialize, Serialize};

/// Tool definition
///
/// Used to define available tools in requests
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    /// Tool specification
    pub tool_specification: ToolSpecification,
}

/// Tool specification
///
/// Defines tool name, description, and input schema
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpecification {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// Input schema (JSON Schema)
    pub input_schema: InputSchema,
}

/// Input schema
///
/// Wraps JSON Schema definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputSchema {
    /// JSON Schema definition
    pub json: serde_json::Value,
}

impl Default for InputSchema {
    fn default() -> Self {
        Self {
            json: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }
}

impl InputSchema {
    /// Create from JSON value
    pub fn from_json(json: serde_json::Value) -> Self {
        Self { json }
    }
}

/// Tool execution result
///
/// Used to return tool execution results
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResult {
    /// Tool use ID (corresponds to tool_use_id in request)
    pub tool_use_id: String,
    /// Result content (array format)
    pub content: Vec<serde_json::Map<String, serde_json::Value>>,
    /// Execution status ("success" or "error")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Whether it's an error
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_error: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl ToolResult {
    /// Create successful tool result
    pub fn success(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        let mut map = serde_json::Map::new();
        map.insert(
            "text".to_string(),
            serde_json::Value::String(content.into()),
        );

        Self {
            tool_use_id: tool_use_id.into(),
            content: vec![map],
            status: Some("success".to_string()),
            is_error: false,
        }
    }

    /// Create error tool result
    pub fn error(tool_use_id: impl Into<String>, error_message: impl Into<String>) -> Self {
        let mut map = serde_json::Map::new();
        map.insert(
            "text".to_string(),
            serde_json::Value::String(error_message.into()),
        );

        Self {
            tool_use_id: tool_use_id.into(),
            content: vec![map],
            status: Some("error".to_string()),
            is_error: true,
        }
    }
}

/// Tool use entry
///
/// Used to record tool calls in history messages
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseEntry {
    /// Tool use ID
    pub tool_use_id: String,
    /// Tool name
    pub name: String,
    /// Tool input parameters
    pub input: serde_json::Value,
}

impl ToolUseEntry {
    /// Create new tool use entry
    pub fn new(tool_use_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            tool_use_id: tool_use_id.into(),
            name: name.into(),
            input: serde_json::json!({}),
        }
    }

    /// Set input parameters
    pub fn with_input(mut self, input: serde_json::Value) -> Self {
        self.input = input;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_result_success() {
        let result = ToolResult::success("tool-123", "Operation completed");

        assert!(!result.is_error);
        assert_eq!(result.status, Some("success".to_string()));
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::error("tool-456", "File not found");

        assert!(result.is_error);
        assert_eq!(result.status, Some("error".to_string()));
    }

    #[test]
    fn test_tool_result_serialize() {
        let result = ToolResult::success("tool-789", "Done");
        let json = serde_json::to_string(&result).unwrap();

        assert!(json.contains("\"toolUseId\":\"tool-789\""));
        assert!(json.contains("\"status\":\"success\""));
        // is_error = false should be skipped
        assert!(!json.contains("isError"));
    }

    #[test]
    fn test_tool_use_entry() {
        let entry = ToolUseEntry::new("use-123", "read_file")
            .with_input(serde_json::json!({"path": "/test.txt"}));

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"toolUseId\":\"use-123\""));
        assert!(json.contains("\"name\":\"read_file\""));
        assert!(json.contains("\"path\":\"/test.txt\""));
    }

    #[test]
    fn test_input_schema_default() {
        let schema = InputSchema::default();
        assert_eq!(schema.json["type"], "object");
    }
}
