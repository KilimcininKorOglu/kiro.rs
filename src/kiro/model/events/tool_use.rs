//! 工具使用事件
//!
//! 处理 toolUseEvent 类型的事件

use serde::Deserialize;

use crate::kiro::parser::error::ParseResult;
use crate::kiro::parser::frame::Frame;

use super::base::{EventPayload, EventType};

/// 工具使用事件
///
/// 包含工具调用的流式数据
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolUseEvent {
    /// 工具名称
    pub name: String,
    /// 工具调用 ID
    pub tool_use_id: String,
    /// 工具输入数据 (JSON 字符串，可能是流式的部分数据)
    #[serde(default)]
    pub input: String,
    /// 是否是最后一个块
    #[serde(default)]
    pub stop: bool,
}

impl EventPayload for ToolUseEvent {
    fn from_frame(frame: &Frame) -> ParseResult<Self> {
        frame.payload_as_json()
    }

    fn event_type() -> EventType {
        EventType::ToolUse
    }
}

impl ToolUseEvent {
    /// 是否完成 (stop = true)
    pub fn is_complete(&self) -> bool {
        self.stop
    }

    /// 获取工具名称
    pub fn name(&self) -> &str {
        &self.name
    }

    /// 获取工具调用 ID
    pub fn tool_use_id(&self) -> &str {
        &self.tool_use_id
    }

    /// 获取输入数据
    pub fn input(&self) -> &str {
        &self.input
    }

    /// 尝试将输入解析为 JSON 值
    pub fn input_as_json(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::from_str(&self.input)
    }

    /// 尝试将输入解析为指定类型
    pub fn input_as<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.input)
    }
}

impl std::fmt::Display for ToolUseEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.stop {
            write!(
                f,
                "ToolUse[{}] (id={}, complete): {}",
                self.name, self.tool_use_id, self.input
            )
        } else {
            write!(
                f,
                "ToolUse[{}] (id={}, partial): {}",
                self.name, self.tool_use_id, self.input
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize() {
        let json = r#"{
            "name": "read_file",
            "toolUseId": "tool_123",
            "input": "{\"path\":\"/test.txt\"}",
            "stop": true
        }"#;
        let event: ToolUseEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.name(), "read_file");
        assert_eq!(event.tool_use_id(), "tool_123");
        assert!(event.is_complete());
    }

    #[test]
    fn test_deserialize_partial() {
        let json = r#"{
            "name": "write_file",
            "toolUseId": "tool_456",
            "input": "{\"path\":",
            "stop": false
        }"#;
        let event: ToolUseEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.name(), "write_file");
        assert!(!event.is_complete());
    }

    #[test]
    fn test_input_as_json() {
        let event = ToolUseEvent {
            name: "test".to_string(),
            tool_use_id: "id".to_string(),
            input: r#"{"key": "value"}"#.to_string(),
            stop: true,
        };
        let json = event.input_as_json().unwrap();
        assert_eq!(json["key"], "value");
    }
}
