//! Conversation type definitions
//!
//! Defines conversation-related types for Kiro API, including messages and history

use serde::{Deserialize, Serialize};

use super::tool::{Tool, ToolResult, ToolUseEntry};

/// Conversation state
///
/// Core structure in Kiro API requests, contains current message and history
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationState {
    /// Agent continuation ID
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_continuation_id: Option<String>,
    /// Agent task type (usually "vibe")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_task_type: Option<String>,
    /// Chat trigger type ("MANUAL" or "AUTO")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chat_trigger_type: Option<String>,
    /// Current message
    pub current_message: CurrentMessage,
    /// Conversation ID
    pub conversation_id: String,
    /// History message list
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Message>,
}

impl ConversationState {
    /// Create new conversation state
    pub fn new(conversation_id: impl Into<String>) -> Self {
        Self {
            agent_continuation_id: None,
            agent_task_type: None,
            chat_trigger_type: None,
            current_message: CurrentMessage::default(),
            conversation_id: conversation_id.into(),
            history: Vec::new(),
        }
    }

    /// Set agent continuation ID
    pub fn with_agent_continuation_id(mut self, id: impl Into<String>) -> Self {
        self.agent_continuation_id = Some(id.into());
        self
    }

    /// Set agent task type
    pub fn with_agent_task_type(mut self, task_type: impl Into<String>) -> Self {
        self.agent_task_type = Some(task_type.into());
        self
    }

    /// Set chat trigger type
    pub fn with_chat_trigger_type(mut self, trigger_type: impl Into<String>) -> Self {
        self.chat_trigger_type = Some(trigger_type.into());
        self
    }

    /// Set current message
    pub fn with_current_message(mut self, message: CurrentMessage) -> Self {
        self.current_message = message;
        self
    }

    /// Add history messages
    pub fn with_history(mut self, history: Vec<Message>) -> Self {
        self.history = history;
        self
    }
}

/// Current message container
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentMessage {
    /// User input message
    pub user_input_message: UserInputMessage,
}

impl CurrentMessage {
    /// Create new current message
    pub fn new(user_input_message: UserInputMessage) -> Self {
        Self { user_input_message }
    }
}

/// User input message
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputMessage {
    /// User input message context
    pub user_input_message_context: UserInputMessageContext,
    /// Message content
    pub content: String,
    /// Model ID
    pub model_id: String,
    /// Image list
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<KiroImage>,
    /// Message origin (usually "AI_EDITOR")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
}

impl UserInputMessage {
    /// Create new user input message
    pub fn new(content: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            user_input_message_context: UserInputMessageContext::default(),
            content: content.into(),
            model_id: model_id.into(),
            images: Vec::new(),
            origin: Some("AI_EDITOR".to_string()),
        }
    }

    /// Set message context
    pub fn with_context(mut self, context: UserInputMessageContext) -> Self {
        self.user_input_message_context = context;
        self
    }

    /// Add images
    pub fn with_images(mut self, images: Vec<KiroImage>) -> Self {
        self.images = images;
        self
    }

    /// Set origin
    pub fn with_origin(mut self, origin: impl Into<String>) -> Self {
        self.origin = Some(origin.into());
        self
    }
}

/// User input message context
///
/// Contains tool definitions and tool execution results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputMessageContext {
    /// Tool execution result list
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_results: Vec<ToolResult>,
    /// Available tool list
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Tool>,
}

impl UserInputMessageContext {
    /// Create new message context
    pub fn new() -> Self {
        Self::default()
    }

    /// Set tool list
    pub fn with_tools(mut self, tools: Vec<Tool>) -> Self {
        self.tools = tools;
        self
    }

    /// Set tool results
    pub fn with_tool_results(mut self, results: Vec<ToolResult>) -> Self {
        self.tool_results = results;
        self
    }
}

/// Kiro image
///
/// Image format used in API
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KiroImage {
    /// Image format ("jpeg", "png", "gif", "webp")
    pub format: String,
    /// Image data source
    pub source: KiroImageSource,
}

impl KiroImage {
    /// Create image from base64 data
    pub fn from_base64(format: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            format: format.into(),
            source: KiroImageSource { bytes: data.into() },
        }
    }
}

/// Kiro image data source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KiroImageSource {
    /// Base64 encoded image data
    pub bytes: String,
}

/// History message
///
/// Can be user message or assistant message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    /// User message
    User(HistoryUserMessage),
    /// Assistant message
    Assistant(HistoryAssistantMessage),
}

#[allow(dead_code)]
impl Message {
    /// Create user message
    pub fn user(content: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self::User(HistoryUserMessage::new(content, model_id))
    }

    /// Create assistant message
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::Assistant(HistoryAssistantMessage::new(content))
    }

    /// Check if user message
    pub fn is_user(&self) -> bool {
        matches!(self, Self::User(_))
    }

    /// Check if assistant message
    pub fn is_assistant(&self) -> bool {
        matches!(self, Self::Assistant(_))
    }
}

/// History user message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryUserMessage {
    /// User input message
    pub user_input_message: UserMessage,
}

impl HistoryUserMessage {
    /// Create new history user message
    pub fn new(content: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            user_input_message: UserMessage::new(content, model_id),
        }
    }
}

/// User message (used in history)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMessage {
    /// Message content
    pub content: String,
    /// Model ID
    pub model_id: String,
    /// Message origin
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// Image list
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<KiroImage>,
    /// User input message context
    #[serde(default, skip_serializing_if = "is_default_context")]
    pub user_input_message_context: UserInputMessageContext,
}

fn is_default_context(ctx: &UserInputMessageContext) -> bool {
    ctx.tools.is_empty() && ctx.tool_results.is_empty()
}

impl UserMessage {
    /// Create new user message
    pub fn new(content: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            model_id: model_id.into(),
            origin: Some("AI_EDITOR".to_string()),
            images: Vec::new(),
            user_input_message_context: UserInputMessageContext::default(),
        }
    }

    /// Set images
    pub fn with_images(mut self, images: Vec<KiroImage>) -> Self {
        self.images = images;
        self
    }

    /// Set context
    pub fn with_context(mut self, context: UserInputMessageContext) -> Self {
        self.user_input_message_context = context;
        self
    }
}

/// History assistant message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryAssistantMessage {
    /// Assistant response message
    pub assistant_response_message: AssistantMessage,
}

impl HistoryAssistantMessage {
    /// Create new history assistant message
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            assistant_response_message: AssistantMessage::new(content),
        }
    }
}

/// Assistant message (used in history)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    /// Response content
    pub content: String,
    /// Tool use list
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_uses: Option<Vec<ToolUseEntry>>,
}

impl AssistantMessage {
    /// Create new assistant message
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            tool_uses: None,
        }
    }

    /// Set tool uses
    pub fn with_tool_uses(mut self, tool_uses: Vec<ToolUseEntry>) -> Self {
        self.tool_uses = Some(tool_uses);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_state_new() {
        let state = ConversationState::new("conv-123")
            .with_agent_task_type("vibe")
            .with_chat_trigger_type("MANUAL");

        assert_eq!(state.conversation_id, "conv-123");
        assert_eq!(state.agent_task_type, Some("vibe".to_string()));
        assert_eq!(state.chat_trigger_type, Some("MANUAL".to_string()));
    }

    #[test]
    fn test_user_input_message() {
        let msg = UserInputMessage::new("Hello", "claude-3-5-sonnet").with_origin("AI_EDITOR");

        assert_eq!(msg.content, "Hello");
        assert_eq!(msg.model_id, "claude-3-5-sonnet");
        assert_eq!(msg.origin, Some("AI_EDITOR".to_string()));
    }

    #[test]
    fn test_message_enum() {
        let user_msg = Message::user("Hello", "model-id");
        assert!(user_msg.is_user());
        assert!(!user_msg.is_assistant());

        let assistant_msg = Message::assistant("Hi there!");
        assert!(assistant_msg.is_assistant());
        assert!(!assistant_msg.is_user());
    }

    #[test]
    fn test_history_serialize() {
        let history = vec![
            Message::user("Hello", "claude-3-5-sonnet"),
            Message::assistant("Hi! How can I help you?"),
        ];

        let json = serde_json::to_string(&history).unwrap();
        assert!(json.contains("userInputMessage"));
        assert!(json.contains("assistantResponseMessage"));
    }

    #[test]
    fn test_conversation_state_serialize() {
        let state = ConversationState::new("conv-123")
            .with_agent_task_type("vibe")
            .with_current_message(CurrentMessage::new(UserInputMessage::new(
                "Hello",
                "claude-3-5-sonnet",
            )));

        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"conversationId\":\"conv-123\""));
        assert!(json.contains("\"agentTaskType\":\"vibe\""));
        assert!(json.contains("\"content\":\"Hello\""));
    }
}
