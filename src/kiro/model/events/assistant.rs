//! 助手响应事件
//!
//! 处理 assistantResponseEvent 类型的事件

use serde::Deserialize;

use crate::kiro::parser::error::ParseResult;
use crate::kiro::parser::frame::Frame;

use super::base::{EventPayload, EventType};

/// 助手响应事件
///
/// 包含 AI 助手的流式响应内容
#[derive(Debug, Clone, Deserialize)]
pub struct AssistantResponseEvent {
    /// 响应内容片段
    #[serde(default)]
    pub content: String,
}

impl EventPayload for AssistantResponseEvent {
    fn from_frame(frame: &Frame) -> ParseResult<Self> {
        frame.payload_as_json()
    }

    fn event_type() -> EventType {
        EventType::AssistantResponse
    }
}

impl AssistantResponseEvent {
    /// 获取内容
    pub fn content(&self) -> &str {
        &self.content
    }

    /// 判断内容是否为空
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// 获取内容长度
    pub fn len(&self) -> usize {
        self.content.len()
    }
}

impl std::fmt::Display for AssistantResponseEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize() {
        let json = r#"{"content":"Hello, world!"}"#;
        let event: AssistantResponseEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.content(), "Hello, world!");
    }

    #[test]
    fn test_deserialize_empty() {
        let json = r#"{}"#;
        let event: AssistantResponseEvent = serde_json::from_str(json).unwrap();
        assert!(event.is_empty());
    }

    #[test]
    fn test_display() {
        let event = AssistantResponseEvent {
            content: "test".to_string(),
        };
        assert_eq!(format!("{}", event), "test");
    }
}
