//! 助手响应事件
//!
//! 处理 assistantResponseEvent 类型的事件

use serde::{Deserialize, Serialize};

use crate::kiro::model::common::{
    CodeQuery, ContentType, Customization, FollowupPrompt, MessageStatus, ProgrammingLanguage,
    Reference, SupplementaryWebLink, UserIntent,
};
use crate::kiro::parser::error::ParseResult;
use crate::kiro::parser::frame::Frame;

use super::base::{EventPayload, EventType};

/// 助手响应事件
///
/// 包含 AI 助手的流式响应内容和元数据
///
/// # 向后兼容性
///
/// 此结构体扩展了原有的简化版本，所有新增字段都是可选的，
/// 确保现有代码继续正常工作。对于流式响应，通常只有 `content` 字段有值。
///
/// # 示例
///
/// ```rust
/// use kiro_rs::kiro::model::events::AssistantResponseEvent;
///
/// // 简单的流式响应（只有 content）
/// let json = r#"{"content":"Hello, world!"}"#;
/// let event: AssistantResponseEvent = serde_json::from_str(json).unwrap();
/// assert_eq!(event.content(), "Hello, world!");
///
/// // 完整响应（包含所有元数据）
/// let full_json = r#"{
///     "content": "Here is the answer",
///     "conversationId": "conv-123",
///     "messageId": "msg-456",
///     "messageStatus": "COMPLETED",
///     "contentType": "text/markdown"
/// }"#;
/// let full_event: AssistantResponseEvent = serde_json::from_str(full_json).unwrap();
/// assert!(full_event.is_completed());
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantResponseEvent {
    // ========== 核心字段 ==========
    /// 响应内容片段
    #[serde(default)]
    pub content: String,

    /// 会话 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,

    /// 消息 ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,

    /// 内容类型（如 text/markdown, text/plain）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<ContentType>,

    /// 消息状态（COMPLETED, IN_PROGRESS, ERROR）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_status: Option<MessageStatus>,

    // ========== 引用和链接字段 ==========
    /// 补充网页链接
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supplementary_web_links: Vec<SupplementaryWebLink>,

    /// 代码引用
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<Reference>,

    /// 代码引用（另一种格式）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub code_reference: Vec<Reference>,

    // ========== 交互字段 ==========
    /// 后续提示
    #[serde(skip_serializing_if = "Option::is_none")]
    pub followup_prompt: Option<FollowupPrompt>,

    // ========== 上下文字段 ==========
    /// 编程语言
    #[serde(skip_serializing_if = "Option::is_none")]
    pub programming_language: Option<ProgrammingLanguage>,

    /// 定制化配置列表
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub customizations: Vec<Customization>,

    /// 用户意图
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_intent: Option<UserIntent>,

    /// 代码查询
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_query: Option<CodeQuery>,
}

impl EventPayload for AssistantResponseEvent {
    fn from_frame(frame: &Frame) -> ParseResult<Self> {
        frame.payload_as_json()
    }

    fn event_type() -> EventType {
        EventType::AssistantResponse
    }
}

impl Default for AssistantResponseEvent {
    fn default() -> Self {
        Self {
            content: String::new(),
            conversation_id: None,
            message_id: None,
            content_type: None,
            message_status: None,
            supplementary_web_links: Vec::new(),
            references: Vec::new(),
            code_reference: Vec::new(),
            followup_prompt: None,
            programming_language: None,
            customizations: Vec::new(),
            user_intent: None,
            code_query: None,
        }
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
    fn test_deserialize_with_followup() {
        let json = r#"{
            "content": "Done",
            "followupPrompt": {
                "content": "Would you like me to explain further?",
                "userIntent": "EXPLAIN_CODE_SELECTION"
            }
        }"#;
        let event: AssistantResponseEvent = serde_json::from_str(json).unwrap();

        assert!(event.followup_prompt.is_some());
        let prompt = event.followup_prompt.unwrap();
        assert_eq!(prompt.content, "Would you like me to explain further?");
        assert_eq!(prompt.user_intent, Some(UserIntent::ExplainCodeSelection));
    }

    #[test]
    fn test_serialize_minimal() {
        // 测试序列化时跳过空字段
        let event = AssistantResponseEvent {
            content: "Test".to_string(),
            ..Default::default()
        };

        let json = serde_json::to_string(&event).unwrap();
        // 应该只包含 content 字段
        assert!(json.contains("\"content\":\"Test\""));
        // 不应该包含空的可选字段
        assert!(!json.contains("conversationId"));
        assert!(!json.contains("supplementaryWebLinks"));
    }

    #[test]
    fn test_display() {
        let event = AssistantResponseEvent {
            content: "test".to_string(),
            ..Default::default()
        };
        assert_eq!(format!("{}", event), "test");
    }
}
