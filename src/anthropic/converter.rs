//! Anthropic → Kiro protocol converter
//!
//! Responsible for converting Anthropic API request format to Kiro API request format

use uuid::Uuid;

use crate::kiro::model::requests::conversation::{
    AssistantMessage, ConversationState, CurrentMessage, HistoryAssistantMessage,
    HistoryUserMessage, KiroImage, Message, UserInputMessage, UserInputMessageContext, UserMessage,
};
use crate::kiro::model::requests::tool::{
    InputSchema, Tool, ToolResult, ToolSpecification, ToolUseEntry,
};

use super::types::{ContentBlock, MessagesRequest};

/// Content appended to the end of Write tool description
const WRITE_TOOL_DESCRIPTION_SUFFIX: &str = "- IMPORTANT: If the content to write exceeds 150 lines, you MUST only write the first 50 lines using this tool, then use `Edit` tool to append the remaining content in chunks of no more than 50 lines each. If needed, leave a unique placeholder to help append content. Do NOT attempt to write all content at once.";

/// Content appended to the end of Edit tool description
const EDIT_TOOL_DESCRIPTION_SUFFIX: &str = "- IMPORTANT: If the `new_string` content exceeds 50 lines, you MUST split it into multiple Edit calls, each replacing no more than 50 lines at a time. If used to append content, leave a unique placeholder to help append content. On the final chunk, do NOT include the placeholder.";

/// Chunked write policy appended to system prompt
const SYSTEM_CHUNKED_POLICY: &str = "\
When the Write or Edit tool has content size limits, always comply silently. \
Never suggest bypassing these limits via alternative tools. \
Never ask the user whether to switch approaches. \
Complete all chunked operations without commentary.";

/// Thinking mode prompt injected into system prompt when thinking is enabled
const THINKING_MODE_PROMPT: &str = "<thinking_mode>enabled</thinking_mode>\n<max_thinking_length>200000</max_thinking_length>";

/// Parse model name and extract thinking mode from suffix
/// Returns (actual_model, thinking_enabled)
pub fn parse_model_and_thinking(model: &str, suffix: &str) -> (String, bool) {
    let model_lower = model.to_lowercase();
    let suffix_lower = suffix.to_lowercase();

    if model_lower.ends_with(&suffix_lower) {
        let actual_model = model[..model.len() - suffix.len()].to_string();
        (actual_model, true)
    } else {
        (model.to_string(), false)
    }
}

/// Inject thinking mode prompt into system prompt
pub fn inject_thinking_prompt(system_prompt: &str) -> String {
    if system_prompt.is_empty() {
        THINKING_MODE_PROMPT.to_string()
    } else {
        format!("{}\n\n{}", THINKING_MODE_PROMPT, system_prompt)
    }
}

/// Model mapping: Map Anthropic model names to Kiro model IDs
///
/// Model mapping with version-specific internal IDs:
/// - Sonnet 4.5 → CLAUDE_SONNET_4_5_20250929_V1_0
/// - Sonnet 4 → CLAUDE_SONNET_4_20250514_V1_0
/// - Sonnet 3.7 → CLAUDE_3_7_SONNET_20250219_V1_0
/// - Other sonnet → claude-sonnet-4.5
/// - Opus 4.5 → claude-opus-4.5
/// - Opus (other) → claude-opus-4.6
/// - Haiku → claude-haiku-4.5
pub fn map_model(model: &str) -> Option<String> {
    let model_lower = model.to_lowercase();

    if model_lower.contains("sonnet") {
        if model_lower.contains("4-5") || model_lower.contains("4.5") {
            Some("CLAUDE_SONNET_4_5_20250929_V1_0".to_string())
        } else if model_lower.contains("sonnet-4") || model_lower.contains("sonnet_4") {
            Some("CLAUDE_SONNET_4_20250514_V1_0".to_string())
        } else if model_lower.contains("3-7") || model_lower.contains("3.7") {
            Some("CLAUDE_3_7_SONNET_20250219_V1_0".to_string())
        } else {
            Some("claude-sonnet-4.5".to_string())
        }
    } else if model_lower.contains("opus") {
        if model_lower.contains("4-5") || model_lower.contains("4.5") {
            Some("claude-opus-4.5".to_string())
        } else {
            Some("claude-opus-4.6".to_string())
        }
    } else if model_lower.contains("haiku") {
        Some("claude-haiku-4.5".to_string())
    } else {
        None
    }
}

/// Conversion result
#[derive(Debug)]
pub struct ConversionResult {
    /// Converted Kiro request
    pub conversation_state: ConversationState,
}

/// Conversion error
#[derive(Debug)]
pub enum ConversionError {
    UnsupportedModel(String),
    EmptyMessages,
}

impl std::fmt::Display for ConversionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConversionError::UnsupportedModel(model) => write!(f, "Model not supported: {}", model),
            ConversionError::EmptyMessages => write!(f, "Message list is empty"),
        }
    }
}

impl std::error::Error for ConversionError {}

/// Extract session UUID from metadata.user_id
///
/// user_id format: user_xxx_account__session_0b4445e1-f5be-49e1-87ce-62bbc28ad705
/// Extract the UUID after session_ as conversationId
fn extract_session_id(user_id: &str) -> Option<String> {
    // Find content after "session_"
    if let Some(pos) = user_id.find("session_") {
        let session_part = &user_id[pos + 8..]; // "session_" length is 8
        // session_part should be UUID format: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
        // Verify if it's a valid UUID format (36 characters, including 4 hyphens)
        if session_part.len() >= 36 {
            let uuid_str = &session_part[..36];
            // Simple UUID format validation
            if uuid_str.chars().filter(|c| *c == '-').count() == 4 {
                return Some(uuid_str.to_string());
            }
        }
    }
    None
}

/// Collect all tool names used in history messages
fn collect_history_tool_names(history: &[Message]) -> Vec<String> {
    let mut tool_names = Vec::new();

    for msg in history {
        if let Message::Assistant(assistant_msg) = msg {
            if let Some(ref tool_uses) = assistant_msg.assistant_response_message.tool_uses {
                for tool_use in tool_uses {
                    if !tool_names.contains(&tool_use.name) {
                        tool_names.push(tool_use.name.clone());
                    }
                }
            }
        }
    }

    tool_names
}

/// Create placeholder definition for tools used in history but not in tools list
/// Kiro API requirement: Tools referenced in history messages must have definitions in currentMessage.tools
fn create_placeholder_tool(name: &str) -> Tool {
    Tool {
        tool_specification: ToolSpecification {
            name: name.to_string(),
            description: "Tool used in conversation history".to_string(),
            input_schema: InputSchema::from_json(serde_json::json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": true
            })),
        },
    }
}

/// Convert Anthropic request to Kiro request
pub fn convert_request(req: &MessagesRequest) -> Result<ConversionResult, ConversionError> {
    // 1. Map model
    let model_id = map_model(&req.model)
        .ok_or_else(|| ConversionError::UnsupportedModel(req.model.clone()))?;

    // 2. Check message list
    if req.messages.is_empty() {
        return Err(ConversionError::EmptyMessages);
    }

    // 3. Generate conversation ID and agent ID
    // Prefer extracting session UUID from metadata.user_id as conversationId
    let conversation_id = req
        .metadata
        .as_ref()
        .and_then(|m| m.user_id.as_ref())
        .and_then(|user_id| extract_session_id(user_id))
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let agent_continuation_id = Uuid::new_v4().to_string();

    // 4. Determine trigger type
    let chat_trigger_type = determine_chat_trigger_type(req);

    // 5. Process last message as current_message
    let last_message = req.messages.last().unwrap();
    let (text_content, images, tool_results) = process_message_content(&last_message.content)?;

    // 6. Convert tool definitions
    let mut tools = convert_tools(&req.tools);

    // 7. Build history messages (need to build first to collect tools used in history)
    let mut history = build_history(req, &model_id)?;

    // 8. Validate and filter tool_use/tool_result pairing
    // Remove orphaned tool_results (without corresponding tool_use)
    // Also return orphaned tool_use_id set for subsequent cleanup
    let (validated_tool_results, orphaned_tool_use_ids) =
        validate_tool_pairing(&history, &tool_results);

    // 9. Remove orphaned tool_uses from history (Kiro API requires tool_use must have corresponding tool_result)
    remove_orphaned_tool_uses(&mut history, &orphaned_tool_use_ids);

    // 10. Collect tool names used in history, generate placeholder definitions for missing tools
    // Kiro API requirement: Tools referenced in history messages must have definitions in tools list
    // Note: Kiro matches tool names case-insensitively, so we also need case-insensitive comparison
    let history_tool_names = collect_history_tool_names(&history);
    let existing_tool_names: std::collections::HashSet<_> = tools
        .iter()
        .map(|t| t.tool_specification.name.to_lowercase())
        .collect();

    for tool_name in history_tool_names {
        if !existing_tool_names.contains(&tool_name.to_lowercase()) {
            tools.push(create_placeholder_tool(&tool_name));
        }
    }

    // 11. Build UserInputMessageContext
    let mut context = UserInputMessageContext::new();
    if !tools.is_empty() {
        context = context.with_tools(tools);
    }
    if !validated_tool_results.is_empty() {
        context = context.with_tool_results(validated_tool_results);
    }

    // 12. Build current message
    // Preserve text content, don't discard user text even if there are tool results
    let content = text_content;

    let mut user_input = UserInputMessage::new(content, &model_id)
        .with_context(context)
        .with_origin("AI_EDITOR");

    if !images.is_empty() {
        user_input = user_input.with_images(images);
    }

    let current_message = CurrentMessage::new(user_input);

    // 13. Build ConversationState
    let conversation_state = ConversationState::new(conversation_id)
        .with_agent_continuation_id(agent_continuation_id)
        .with_agent_task_type("vibe")
        .with_chat_trigger_type(chat_trigger_type)
        .with_current_message(current_message)
        .with_history(history);

    Ok(ConversionResult { conversation_state })
}

/// Determine chat trigger type
/// "AUTO" mode may cause 400 Bad Request errors
fn determine_chat_trigger_type(_req: &MessagesRequest) -> String {
    "MANUAL".to_string()
}

/// Process message content, extract text, images and tool results
fn process_message_content(
    content: &serde_json::Value,
) -> Result<(String, Vec<KiroImage>, Vec<ToolResult>), ConversionError> {
    let mut text_parts = Vec::new();
    let mut images = Vec::new();
    let mut tool_results = Vec::new();

    match content {
        serde_json::Value::String(s) => {
            text_parts.push(s.clone());
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Ok(block) = serde_json::from_value::<ContentBlock>(item.clone()) {
                    match block.block_type.as_str() {
                        "text" => {
                            if let Some(text) = block.text {
                                text_parts.push(text);
                            }
                        }
                        "image" => {
                            if let Some(source) = block.source {
                                if let Some(format) = get_image_format(&source.media_type) {
                                    images.push(KiroImage::from_base64(format, source.data));
                                }
                            }
                        }
                        "tool_result" => {
                            if let Some(tool_use_id) = block.tool_use_id {
                                let result_content = extract_tool_result_content(&block.content);
                                let is_error = block.is_error.unwrap_or(false);

                                let mut result = if is_error {
                                    ToolResult::error(&tool_use_id, result_content)
                                } else {
                                    ToolResult::success(&tool_use_id, result_content)
                                };
                                result.status =
                                    Some(if is_error { "error" } else { "success" }.to_string());

                                tool_results.push(result);
                            }
                        }
                        "tool_use" => {
                            // tool_use is handled in assistant messages, ignored here
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }

    Ok((text_parts.join("\n"), images, tool_results))
}

/// Get image format from media_type
fn get_image_format(media_type: &str) -> Option<String> {
    match media_type {
        "image/jpeg" => Some("jpeg".to_string()),
        "image/png" => Some("png".to_string()),
        "image/gif" => Some("gif".to_string()),
        "image/webp" => Some("webp".to_string()),
        _ => None,
    }
}

/// Extract tool result content
fn extract_tool_result_content(content: &Option<serde_json::Value>) -> String {
    match content {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(arr)) => {
            let mut parts = Vec::new();
            for item in arr {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    parts.push(text.to_string());
                }
            }
            parts.join("\n")
        }
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

/// Validate and filter tool_use/tool_result pairing
///
/// Collect all tool_use_ids, validate if tool_results match
/// Silently skip orphaned tool_uses and tool_results, output warning logs
///
/// # Arguments
/// * `history` - History messages reference
/// * `tool_results` - tool_result list in current message
///
/// # Returns
/// Tuple: (validated and filtered tool_result list, orphaned tool_use_id set)
fn validate_tool_pairing(
    history: &[Message],
    tool_results: &[ToolResult],
) -> (Vec<ToolResult>, std::collections::HashSet<String>) {
    use std::collections::HashSet;

    // 1. Collect all tool_use_ids from history
    let mut all_tool_use_ids: HashSet<String> = HashSet::new();
    // 2. Collect tool_use_ids that already have tool_results in history
    let mut history_tool_result_ids: HashSet<String> = HashSet::new();

    for msg in history {
        match msg {
            Message::Assistant(assistant_msg) => {
                if let Some(ref tool_uses) = assistant_msg.assistant_response_message.tool_uses {
                    for tool_use in tool_uses {
                        all_tool_use_ids.insert(tool_use.tool_use_id.clone());
                    }
                }
            }
            Message::User(user_msg) => {
                // Collect tool_results from history user messages
                for result in &user_msg
                    .user_input_message
                    .user_input_message_context
                    .tool_results
                {
                    history_tool_result_ids.insert(result.tool_use_id.clone());
                }
            }
        }
    }

    // 3. Calculate truly unpaired tool_use_ids (excluding those already paired in history)
    let mut unpaired_tool_use_ids: HashSet<String> = all_tool_use_ids
        .difference(&history_tool_result_ids)
        .cloned()
        .collect();

    // 4. Filter and validate current message's tool_results
    let mut filtered_results = Vec::new();

    for result in tool_results {
        if unpaired_tool_use_ids.contains(&result.tool_use_id) {
            // Pairing successful
            filtered_results.push(result.clone());
            unpaired_tool_use_ids.remove(&result.tool_use_id);
        } else if all_tool_use_ids.contains(&result.tool_use_id) {
            // tool_use exists but already paired in history, this is a duplicate tool_result
            tracing::warn!(
                "Skipping duplicate tool_result: tool_use already paired in history, tool_use_id={}",
                result.tool_use_id
            );
        } else {
            // Orphaned tool_result - no corresponding tool_use found
            tracing::warn!(
                "Skipping orphaned tool_result: no corresponding tool_use found, tool_use_id={}",
                result.tool_use_id
            );
        }
    }

    // 5. Detect truly orphaned tool_uses (has tool_use but no tool_result in history or current message)
    for orphaned_id in &unpaired_tool_use_ids {
        tracing::warn!(
            "Detected orphaned tool_use: no corresponding tool_result found, will be removed from history, tool_use_id={}",
            orphaned_id
        );
    }

    (filtered_results, unpaired_tool_use_ids)
}

/// Remove orphaned tool_uses from history messages
///
/// Kiro API requires each tool_use must have a corresponding tool_result, otherwise returns 400 Bad Request.
/// This function iterates through assistant messages in history, removing tool_uses without corresponding tool_results.
///
/// # Arguments
/// * `history` - Mutable history message list
/// * `orphaned_ids` - Set of orphaned tool_use_ids to remove
fn remove_orphaned_tool_uses(
    history: &mut [Message],
    orphaned_ids: &std::collections::HashSet<String>,
) {
    if orphaned_ids.is_empty() {
        return;
    }

    for msg in history.iter_mut() {
        if let Message::Assistant(assistant_msg) = msg {
            if let Some(ref mut tool_uses) = assistant_msg.assistant_response_message.tool_uses {
                let original_len = tool_uses.len();
                tool_uses.retain(|tu| !orphaned_ids.contains(&tu.tool_use_id));

                // If empty after removal, set to None
                if tool_uses.is_empty() {
                    assistant_msg.assistant_response_message.tool_uses = None;
                } else if tool_uses.len() != original_len {
                    tracing::debug!(
                        "Removed {} orphaned tool_uses from assistant message",
                        original_len - tool_uses.len()
                    );
                }
            }
        }
    }
}

/// Convert tool definitions
fn convert_tools(tools: &Option<Vec<super::types::Tool>>) -> Vec<Tool> {
    let Some(tools) = tools else {
        return Vec::new();
    };

    tools
        .iter()
        .map(|t| {
            let mut description = t.description.clone();

            // Append custom description suffix for Write/Edit tools
            let suffix = match t.name.as_str() {
                "Write" => WRITE_TOOL_DESCRIPTION_SUFFIX,
                "Edit" => EDIT_TOOL_DESCRIPTION_SUFFIX,
                _ => "",
            };
            if !suffix.is_empty() {
                description.push('\n');
                description.push_str(suffix);
            }

            // Limit description length to 10000 characters (safe UTF-8 truncation, single pass)
            let description = match description.char_indices().nth(10000) {
                Some((idx, _)) => description[..idx].to_string(),
                None => description,
            };

            Tool {
                tool_specification: ToolSpecification {
                    name: t.name.clone(),
                    description,
                    input_schema: InputSchema::from_json(serde_json::json!(t.input_schema)),
                },
            }
        })
        .collect()
}

/// Generate thinking tag prefix
fn generate_thinking_prefix(req: &MessagesRequest) -> Option<String> {
    if let Some(t) = &req.thinking {
        if t.thinking_type == "enabled" {
            return Some(format!(
                "<thinking_mode>enabled</thinking_mode><max_thinking_length>{}</max_thinking_length>",
                t.budget_tokens
            ));
        } else if t.thinking_type == "adaptive" {
            let effort = req
                .output_config
                .as_ref()
                .map(|c| c.effort.as_str())
                .unwrap_or("high");
            return Some(format!(
                "<thinking_mode>adaptive</thinking_mode><thinking_effort>{}</thinking_effort>",
                effort
            ));
        }
    }
    None
}

/// Check if content already contains thinking tags
fn has_thinking_tags(content: &str) -> bool {
    content.contains("<thinking_mode>") || content.contains("<max_thinking_length>")
}

/// Build history messages
fn build_history(req: &MessagesRequest, model_id: &str) -> Result<Vec<Message>, ConversionError> {
    let mut history = Vec::new();

    // Generate thinking prefix (if needed)
    let thinking_prefix = generate_thinking_prefix(req);

    // 1. Process system messages
    if let Some(ref system) = req.system {
        let system_content: String = system
            .iter()
            .map(|s| s.text.clone())
            .collect::<Vec<_>>()
            .join("\n");

        if !system_content.is_empty() {
            // Append chunked write policy to system message
            let system_content = format!("{}\n{}", system_content, SYSTEM_CHUNKED_POLICY);

            // Inject thinking tags at the beginning of system message (if needed and not present)
            let final_content = if let Some(ref prefix) = thinking_prefix {
                if !has_thinking_tags(&system_content) {
                    format!("{}\n{}", prefix, system_content)
                } else {
                    system_content
                }
            } else {
                system_content
            };

            // System message as user + assistant pair
            let user_msg = HistoryUserMessage::new(final_content, model_id);
            history.push(Message::User(user_msg));

            let assistant_msg = HistoryAssistantMessage::new("I will follow these instructions.");
            history.push(Message::Assistant(assistant_msg));
        }
    } else if let Some(ref prefix) = thinking_prefix {
        // No system message but has thinking config, insert new system message
        let user_msg = HistoryUserMessage::new(prefix.clone(), model_id);
        history.push(Message::User(user_msg));

        let assistant_msg = HistoryAssistantMessage::new("I will follow these instructions.");
        history.push(Message::Assistant(assistant_msg));
    }

    // 2. Process regular message history
    // Last message is used as currentMessage, not added to history
    let history_end_index = req.messages.len().saturating_sub(1);

    // If last message is assistant, include it in history
    let last_is_assistant = req
        .messages
        .last()
        .map(|m| m.role == "assistant")
        .unwrap_or(false);

    let history_end_index = if last_is_assistant {
        req.messages.len()
    } else {
        history_end_index
    };

    // Collect and pair messages
    let mut user_buffer: Vec<&super::types::Message> = Vec::new();

    for i in 0..history_end_index {
        let msg = &req.messages[i];

        if msg.role == "user" {
            user_buffer.push(msg);
        } else if msg.role == "assistant" {
            // Encountered assistant, process accumulated user messages
            if !user_buffer.is_empty() {
                let merged_user = merge_user_messages(&user_buffer, model_id)?;
                history.push(Message::User(merged_user));
                user_buffer.clear();

                // Add assistant message
                let assistant = convert_assistant_message(msg)?;
                history.push(Message::Assistant(assistant));
            }
        }
    }

    // Handle trailing orphaned user messages
    if !user_buffer.is_empty() {
        let merged_user = merge_user_messages(&user_buffer, model_id)?;
        history.push(Message::User(merged_user));

        // Auto-pair with an "OK" assistant response
        let auto_assistant = HistoryAssistantMessage::new("OK");
        history.push(Message::Assistant(auto_assistant));
    }

    Ok(history)
}

/// Merge multiple user messages
fn merge_user_messages(
    messages: &[&super::types::Message],
    model_id: &str,
) -> Result<HistoryUserMessage, ConversionError> {
    let mut content_parts = Vec::new();
    let mut all_images = Vec::new();
    let mut all_tool_results = Vec::new();

    for msg in messages {
        let (text, images, tool_results) = process_message_content(&msg.content)?;
        if !text.is_empty() {
            content_parts.push(text);
        }
        all_images.extend(images);
        all_tool_results.extend(tool_results);
    }

    let content = content_parts.join("\n");
    // Preserve text content, don't discard user text even if there are tool results
    let mut user_msg = UserMessage::new(&content, model_id);

    if !all_images.is_empty() {
        user_msg = user_msg.with_images(all_images);
    }

    if !all_tool_results.is_empty() {
        let mut ctx = UserInputMessageContext::new();
        ctx = ctx.with_tool_results(all_tool_results);
        user_msg = user_msg.with_context(ctx);
    }

    Ok(HistoryUserMessage {
        user_input_message: user_msg,
    })
}

/// Convert assistant message
fn convert_assistant_message(
    msg: &super::types::Message,
) -> Result<HistoryAssistantMessage, ConversionError> {
    let mut thinking_content = String::new();
    let mut text_content = String::new();
    let mut tool_uses = Vec::new();

    match &msg.content {
        serde_json::Value::String(s) => {
            text_content = s.clone();
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Ok(block) = serde_json::from_value::<ContentBlock>(item.clone()) {
                    match block.block_type.as_str() {
                        "thinking" => {
                            if let Some(thinking) = block.thinking {
                                thinking_content.push_str(&thinking);
                            }
                        }
                        "text" => {
                            if let Some(text) = block.text {
                                text_content.push_str(&text);
                            }
                        }
                        "tool_use" => {
                            if let (Some(id), Some(name)) = (block.id, block.name) {
                                let input = block.input.unwrap_or(serde_json::json!({}));
                                tool_uses.push(ToolUseEntry::new(id, name).with_input(input));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }

    // Combine thinking and text content
    // Format: <thinking>thinking content</thinking>\n\ntext content
    // Note: Kiro API requires content field cannot be empty, need placeholder when only tool_use
    let final_content = if !thinking_content.is_empty() {
        if !text_content.is_empty() {
            format!(
                "<thinking>{}</thinking>\n\n{}",
                thinking_content, text_content
            )
        } else {
            format!("<thinking>{}</thinking>", thinking_content)
        }
    } else if text_content.is_empty() && !tool_uses.is_empty() {
        " ".to_string()
    } else {
        text_content
    };

    let mut assistant = AssistantMessage::new(final_content);
    if !tool_uses.is_empty() {
        assistant = assistant.with_tool_uses(tool_uses);
    }

    Ok(HistoryAssistantMessage {
        assistant_response_message: assistant,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_model_sonnet_4() {
        // Sonnet 4 should map to CLAUDE_SONNET_4_20250514_V1_0
        assert_eq!(
            map_model("claude-sonnet-4-20250514"),
            Some("CLAUDE_SONNET_4_20250514_V1_0".to_string())
        );
    }

    #[test]
    fn test_map_model_sonnet_4_5() {
        // Sonnet 4.5 should map to CLAUDE_SONNET_4_5_20250929_V1_0
        assert_eq!(
            map_model("claude-sonnet-4-5-20250929"),
            Some("CLAUDE_SONNET_4_5_20250929_V1_0".to_string())
        );
    }

    #[test]
    fn test_map_model_sonnet_3_7() {
        // Sonnet 3.7 should map to CLAUDE_3_7_SONNET_20250219_V1_0
        assert_eq!(
            map_model("claude-3-7-sonnet-20250219"),
            Some("CLAUDE_3_7_SONNET_20250219_V1_0".to_string())
        );
    }

    #[test]
    fn test_map_model_sonnet_legacy() {
        // Legacy sonnet (3.5) should fallback to claude-sonnet-4.5
        assert_eq!(
            map_model("claude-3-5-sonnet-20241022"),
            Some("claude-sonnet-4.5".to_string())
        );
    }

    #[test]
    fn test_map_model_opus() {
        assert!(
            map_model("claude-opus-4-20250514")
                .unwrap()
                .contains("opus")
        );
    }

    #[test]
    fn test_map_model_haiku() {
        assert!(
            map_model("claude-haiku-4-20250514")
                .unwrap()
                .contains("haiku")
        );
    }

    #[test]
    fn test_map_model_unsupported() {
        assert!(map_model("gpt-4").is_none());
    }

    #[test]
    fn test_map_model_thinking_suffix_sonnet() {
        // thinking suffix should not affect sonnet 4.5 model mapping
        let result = map_model("claude-sonnet-4-5-20250929-thinking");
        assert_eq!(result, Some("CLAUDE_SONNET_4_5_20250929_V1_0".to_string()));
    }

    #[test]
    fn test_map_model_thinking_suffix_opus_4_5() {
        // thinking suffix should not affect opus 4.5 model mapping
        let result = map_model("claude-opus-4-5-20251101-thinking");
        assert_eq!(result, Some("claude-opus-4.5".to_string()));
    }

    #[test]
    fn test_map_model_thinking_suffix_opus_4_6() {
        // thinking suffix should not affect opus 4.6 model mapping
        let result = map_model("claude-opus-4-6-thinking");
        assert_eq!(result, Some("claude-opus-4.6".to_string()));
    }

    #[test]
    fn test_map_model_thinking_suffix_haiku() {
        // thinking suffix should not affect haiku model mapping
        let result = map_model("claude-haiku-4-5-20251001-thinking");
        assert_eq!(result, Some("claude-haiku-4.5".to_string()));
    }

    #[test]
    fn test_parse_model_and_thinking_with_suffix() {
        let (model, thinking) = parse_model_and_thinking("claude-sonnet-4.5-thinking", "-thinking");
        assert_eq!(model, "claude-sonnet-4.5");
        assert!(thinking);
    }

    #[test]
    fn test_parse_model_and_thinking_without_suffix() {
        let (model, thinking) = parse_model_and_thinking("claude-sonnet-4.5", "-thinking");
        assert_eq!(model, "claude-sonnet-4.5");
        assert!(!thinking);
    }

    #[test]
    fn test_parse_model_and_thinking_custom_suffix() {
        let (model, thinking) = parse_model_and_thinking("claude-opus-4.5-think", "-think");
        assert_eq!(model, "claude-opus-4.5");
        assert!(thinking);
    }

    #[test]
    fn test_parse_model_and_thinking_case_insensitive() {
        let (model, thinking) = parse_model_and_thinking("claude-sonnet-4.5-THINKING", "-thinking");
        assert_eq!(model, "claude-sonnet-4.5");
        assert!(thinking);
    }

    #[test]
    fn test_inject_thinking_prompt_empty() {
        let result = inject_thinking_prompt("");
        assert!(result.contains("<thinking_mode>enabled</thinking_mode>"));
    }

    #[test]
    fn test_inject_thinking_prompt_with_content() {
        let result = inject_thinking_prompt("You are a helpful assistant.");
        assert!(result.starts_with("<thinking_mode>enabled</thinking_mode>"));
        assert!(result.contains("You are a helpful assistant."));
    }

    #[test]
    fn test_determine_chat_trigger_type() {
        // Returns MANUAL when no tools
        let req = MessagesRequest {
            model: "claude-sonnet-4".to_string(),
            max_tokens: 1024,
            messages: vec![],
            stream: false,
            system: None,
            tools: None,
            tool_choice: None,
            thinking: None,
            output_config: None,
            metadata: None,
        };
        assert_eq!(determine_chat_trigger_type(&req), "MANUAL");
    }

    #[test]
    fn test_collect_history_tool_names() {
        use crate::kiro::model::requests::tool::ToolUseEntry;

        // Create history messages containing tool usage
        let mut assistant_msg = AssistantMessage::new("I'll read the file.");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("tool-1", "read")
                .with_input(serde_json::json!({"path": "/test.txt"})),
            ToolUseEntry::new("tool-2", "write")
                .with_input(serde_json::json!({"path": "/out.txt"})),
        ]);

        let history = vec![
            Message::User(HistoryUserMessage::new(
                "Read the file",
                "claude-sonnet-4.5",
            )),
            Message::Assistant(HistoryAssistantMessage {
                assistant_response_message: assistant_msg,
            }),
        ];

        let tool_names = collect_history_tool_names(&history);
        assert_eq!(tool_names.len(), 2);
        assert!(tool_names.contains(&"read".to_string()));
        assert!(tool_names.contains(&"write".to_string()));
    }

    #[test]
    fn test_create_placeholder_tool() {
        let tool = create_placeholder_tool("my_custom_tool");

        assert_eq!(tool.tool_specification.name, "my_custom_tool");
        assert!(!tool.tool_specification.description.is_empty());

        // Verify JSON serialization is correct
        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("\"name\":\"my_custom_tool\""));
    }

    #[test]
    fn test_history_tools_added_to_tools_list() {
        use super::super::types::Message as AnthropicMessage;

        // Create a request with tool usage in history but empty tools list
        let req = MessagesRequest {
            model: "claude-sonnet-4".to_string(),
            max_tokens: 1024,
            messages: vec![
                AnthropicMessage {
                    role: "user".to_string(),
                    content: serde_json::json!("Read the file"),
                },
                AnthropicMessage {
                    role: "assistant".to_string(),
                    content: serde_json::json!([
                        {"type": "text", "text": "I'll read the file."},
                        {"type": "tool_use", "id": "tool-1", "name": "read", "input": {"path": "/test.txt"}}
                    ]),
                },
                AnthropicMessage {
                    role: "user".to_string(),
                    content: serde_json::json!([
                        {"type": "tool_result", "tool_use_id": "tool-1", "content": "file content"}
                    ]),
                },
            ],
            stream: false,
            system: None,
            tools: None, // No tool definitions provided
            tool_choice: None,
            thinking: None,
            output_config: None,
            metadata: None,
        };

        let result = convert_request(&req).unwrap();

        // Verify tools list contains placeholder definitions for tools used in history
        let tools = &result
            .conversation_state
            .current_message
            .user_input_message
            .user_input_message_context
            .tools;

        assert!(!tools.is_empty(), "tools list should not be empty");
        assert!(
            tools.iter().any(|t| t.tool_specification.name == "read"),
            "tools list should contain placeholder definition for 'read' tool"
        );
    }

    #[test]
    fn test_extract_session_id_valid() {
        // Test valid user_id format
        let user_id = "user_0dede55c6dcc4a11a30bbb5e7f22e6fdf86cdeba3820019cc27612af4e1243cd_account__session_8bb5523b-ec7c-4540-a9ca-beb6d79f1552";
        let session_id = extract_session_id(user_id);
        assert_eq!(
            session_id,
            Some("8bb5523b-ec7c-4540-a9ca-beb6d79f1552".to_string())
        );
    }

    #[test]
    fn test_extract_session_id_no_session() {
        // Test user_id without session
        let user_id = "user_0dede55c6dcc4a11a30bbb5e7f22e6fdf86cdeba3820019cc27612af4e1243cd";
        let session_id = extract_session_id(user_id);
        assert_eq!(session_id, None);
    }

    #[test]
    fn test_extract_session_id_invalid_uuid() {
        // Test invalid UUID format
        let user_id = "user_xxx_session_invalid-uuid";
        let session_id = extract_session_id(user_id);
        assert_eq!(session_id, None);
    }

    #[test]
    fn test_convert_request_with_session_metadata() {
        use super::super::types::{Message as AnthropicMessage, Metadata};

        // Test request with metadata, should use session UUID as conversationId
        let req = MessagesRequest {
            model: "claude-sonnet-4".to_string(),
            max_tokens: 1024,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: serde_json::json!("Hello"),
            }],
            stream: false,
            system: None,
            tools: None,
            tool_choice: None,
            thinking: None,
            output_config: None,
            metadata: Some(Metadata {
                user_id: Some(
                    "user_0dede55c6dcc4a11a30bbb5e7f22e6fdf86cdeba3820019cc27612af4e1243cd_account__session_a0662283-7fd3-4399-a7eb-52b9a717ae88".to_string(),
                ),
            }),
        };

        let result = convert_request(&req).unwrap();
        assert_eq!(
            result.conversation_state.conversation_id,
            "a0662283-7fd3-4399-a7eb-52b9a717ae88"
        );
    }

    #[test]
    fn test_convert_request_without_metadata() {
        use super::super::types::Message as AnthropicMessage;

        // Test request without metadata, should generate new UUID
        let req = MessagesRequest {
            model: "claude-sonnet-4".to_string(),
            max_tokens: 1024,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: serde_json::json!("Hello"),
            }],
            stream: false,
            system: None,
            tools: None,
            tool_choice: None,
            thinking: None,
            output_config: None,
            metadata: None,
        };

        let result = convert_request(&req).unwrap();
        // Verify generated UUID format is valid
        assert_eq!(result.conversation_state.conversation_id.len(), 36);
        assert_eq!(
            result
                .conversation_state
                .conversation_id
                .chars()
                .filter(|c| *c == '-')
                .count(),
            4
        );
    }

    #[test]
    fn test_validate_tool_pairing_orphaned_result() {
        // Test orphaned tool_result is filtered
        // No tool_use in history, but tool_results has tool_result
        let history = vec![
            Message::User(HistoryUserMessage::new("Hello", "claude-sonnet-4.5")),
            Message::Assistant(HistoryAssistantMessage::new("Hi there!")),
        ];

        let tool_results = vec![ToolResult::success("orphan-123", "some result")];

        let (filtered, _) = validate_tool_pairing(&history, &tool_results);

        // Orphaned tool_result should be filtered out
        assert!(filtered.is_empty(), "Orphaned tool_result should be filtered");
    }

    #[test]
    fn test_validate_tool_pairing_orphaned_use() {
        use crate::kiro::model::requests::tool::ToolUseEntry;

        // Test orphaned tool_use (has tool_use but no corresponding tool_result)
        let mut assistant_msg = AssistantMessage::new("I'll read the file.");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("tool-orphan", "read")
                .with_input(serde_json::json!({"path": "/test.txt"})),
        ]);

        let history = vec![
            Message::User(HistoryUserMessage::new(
                "Read the file",
                "claude-sonnet-4.5",
            )),
            Message::Assistant(HistoryAssistantMessage {
                assistant_response_message: assistant_msg,
            }),
        ];

        // No tool_result
        let tool_results: Vec<ToolResult> = vec![];

        let (filtered, orphaned) = validate_tool_pairing(&history, &tool_results);

        // Result should be empty (because no tool_result)
        // Should also return orphaned tool_use_id
        assert!(filtered.is_empty());
        assert!(orphaned.contains("tool-orphan"));
    }

    #[test]
    fn test_validate_tool_pairing_valid() {
        use crate::kiro::model::requests::tool::ToolUseEntry;

        // Test normal pairing case
        let mut assistant_msg = AssistantMessage::new("I'll read the file.");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("tool-1", "read")
                .with_input(serde_json::json!({"path": "/test.txt"})),
        ]);

        let history = vec![
            Message::User(HistoryUserMessage::new(
                "Read the file",
                "claude-sonnet-4.5",
            )),
            Message::Assistant(HistoryAssistantMessage {
                assistant_response_message: assistant_msg,
            }),
        ];

        let tool_results = vec![ToolResult::success("tool-1", "file content")];

        let (filtered, orphaned) = validate_tool_pairing(&history, &tool_results);

        // Pairing successful, should be kept, no orphans
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].tool_use_id, "tool-1");
        assert!(orphaned.is_empty());
    }

    #[test]
    fn test_validate_tool_pairing_mixed() {
        use crate::kiro::model::requests::tool::ToolUseEntry;

        // Test mixed case: some paired successfully, some orphaned
        let mut assistant_msg = AssistantMessage::new("I'll use two tools.");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("tool-1", "read").with_input(serde_json::json!({})),
            ToolUseEntry::new("tool-2", "write").with_input(serde_json::json!({})),
        ]);

        let history = vec![
            Message::User(HistoryUserMessage::new("Do something", "claude-sonnet-4.5")),
            Message::Assistant(HistoryAssistantMessage {
                assistant_response_message: assistant_msg,
            }),
        ];

        // tool_results: tool-1 paired, tool-3 orphaned
        let tool_results = vec![
            ToolResult::success("tool-1", "result 1"),
            ToolResult::success("tool-3", "orphan result"), // orphaned
        ];

        let (filtered, orphaned) = validate_tool_pairing(&history, &tool_results);

        // Only tool-1 should be kept
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].tool_use_id, "tool-1");
        // tool-2 is orphaned tool_use (no result), tool-3 is orphaned tool_result
        assert!(orphaned.contains("tool-2"));
    }

    #[test]
    fn test_validate_tool_pairing_history_already_paired() {
        use crate::kiro::model::requests::tool::ToolUseEntry;

        // Test tool_use already paired in history should not be reported as orphaned
        // Scenario: In multi-turn conversation, previous tool_use already has corresponding tool_result in history
        let mut assistant_msg1 = AssistantMessage::new("I'll read the file.");
        assistant_msg1 = assistant_msg1.with_tool_uses(vec![
            ToolUseEntry::new("tool-1", "read")
                .with_input(serde_json::json!({"path": "/test.txt"})),
        ]);

        // Build user message in history containing tool_result
        let mut user_msg_with_result = UserMessage::new("", "claude-sonnet-4.5");
        let mut ctx = UserInputMessageContext::new();
        ctx = ctx.with_tool_results(vec![ToolResult::success("tool-1", "file content")]);
        user_msg_with_result = user_msg_with_result.with_context(ctx);

        let history = vec![
            // Round 1: User request
            Message::User(HistoryUserMessage::new(
                "Read the file",
                "claude-sonnet-4.5",
            )),
            // Round 1: Assistant uses tool
            Message::Assistant(HistoryAssistantMessage {
                assistant_response_message: assistant_msg1,
            }),
            // Round 2: User returns tool result (already paired in history)
            Message::User(HistoryUserMessage {
                user_input_message: user_msg_with_result,
            }),
            // Round 2: Assistant response
            Message::Assistant(HistoryAssistantMessage::new("The file contains...")),
        ];

        // Current message has no tool_results (user just continues conversation)
        let tool_results: Vec<ToolResult> = vec![];

        let (filtered, orphaned) = validate_tool_pairing(&history, &tool_results);

        // Result should be empty, and no orphaned tool_use
        // Because tool-1 is already paired in history
        assert!(filtered.is_empty());
        assert!(orphaned.is_empty());
    }

    #[test]
    fn test_validate_tool_pairing_duplicate_result() {
        use crate::kiro::model::requests::tool::ToolUseEntry;

        // Test duplicate tool_result (already paired in history, current message sends same tool_result again)
        let mut assistant_msg = AssistantMessage::new("I'll read the file.");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("tool-1", "read")
                .with_input(serde_json::json!({"path": "/test.txt"})),
        ]);

        // History already has tool_result
        let mut user_msg_with_result = UserMessage::new("", "claude-sonnet-4.5");
        let mut ctx = UserInputMessageContext::new();
        ctx = ctx.with_tool_results(vec![ToolResult::success("tool-1", "file content")]);
        user_msg_with_result = user_msg_with_result.with_context(ctx);

        let history = vec![
            Message::User(HistoryUserMessage::new(
                "Read the file",
                "claude-sonnet-4.5",
            )),
            Message::Assistant(HistoryAssistantMessage {
                assistant_response_message: assistant_msg,
            }),
            Message::User(HistoryUserMessage {
                user_input_message: user_msg_with_result,
            }),
            Message::Assistant(HistoryAssistantMessage::new("Done")),
        ];

        // Current message sends same tool_result again (duplicate)
        let tool_results = vec![ToolResult::success("tool-1", "file content again")];

        let (filtered, _) = validate_tool_pairing(&history, &tool_results);

        // Duplicate tool_result should be filtered out
        assert!(filtered.is_empty(), "Duplicate tool_result should be filtered");
    }

    #[test]
    fn test_convert_assistant_message_tool_use_only() {
        use super::super::types::Message as AnthropicMessage;

        // Test assistant message containing only tool_use (no text block)
        // Kiro API requires content field cannot be empty
        let msg = AnthropicMessage {
            role: "assistant".to_string(),
            content: serde_json::json!([
                {"type": "tool_use", "id": "toolu_01ABC", "name": "read_file", "input": {"path": "/test.txt"}}
            ]),
        };

        let result = convert_assistant_message(&msg).expect("Should convert successfully");

        // Verify content is not empty (uses placeholder)
        assert!(
            !result.assistant_response_message.content.is_empty(),
            "content should not be empty"
        );
        assert_eq!(
            result.assistant_response_message.content, " ",
            "Should use ' ' placeholder when only tool_use"
        );

        // Verify tool_uses are correctly preserved
        let tool_uses = result
            .assistant_response_message
            .tool_uses
            .expect("Should have tool_uses");
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0].tool_use_id, "toolu_01ABC");
        assert_eq!(tool_uses[0].name, "read_file");
    }

    #[test]
    fn test_convert_assistant_message_with_text_and_tool_use() {
        use super::super::types::Message as AnthropicMessage;

        // Test assistant message containing both text and tool_use
        let msg = AnthropicMessage {
            role: "assistant".to_string(),
            content: serde_json::json!([
                {"type": "text", "text": "Let me read that file for you."},
                {"type": "tool_use", "id": "toolu_02XYZ", "name": "read_file", "input": {"path": "/data.json"}}
            ]),
        };

        let result = convert_assistant_message(&msg).expect("Should convert successfully");

        // Verify content uses original text (not placeholder)
        assert_eq!(
            result.assistant_response_message.content,
            "Let me read that file for you."
        );

        // Verify tool_uses are correctly preserved
        let tool_uses = result
            .assistant_response_message
            .tool_uses
            .expect("Should have tool_uses");
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0].tool_use_id, "toolu_02XYZ");
    }

    #[test]
    fn test_remove_orphaned_tool_uses() {
        use crate::kiro::model::requests::tool::ToolUseEntry;

        // Test removing orphaned tool_uses from history
        let mut assistant_msg = AssistantMessage::new("I'll use multiple tools.");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("tool-1", "read").with_input(serde_json::json!({})),
            ToolUseEntry::new("tool-2", "write").with_input(serde_json::json!({})),
            ToolUseEntry::new("tool-3", "delete").with_input(serde_json::json!({})),
        ]);

        let mut history = vec![
            Message::User(HistoryUserMessage::new("Do something", "claude-sonnet-4.5")),
            Message::Assistant(HistoryAssistantMessage {
                assistant_response_message: assistant_msg,
            }),
        ];

        // Remove tool-1 and tool-3
        let mut orphaned = std::collections::HashSet::new();
        orphaned.insert("tool-1".to_string());
        orphaned.insert("tool-3".to_string());

        remove_orphaned_tool_uses(&mut history, &orphaned);

        // Verify only tool-2 remains
        if let Message::Assistant(ref assistant_msg) = history[1] {
            let tool_uses = assistant_msg
                .assistant_response_message
                .tool_uses
                .as_ref()
                .expect("Should still have tool_uses");
            assert_eq!(tool_uses.len(), 1);
            assert_eq!(tool_uses[0].tool_use_id, "tool-2");
        } else {
            panic!("Should be Assistant message");
        }
    }

    #[test]
    fn test_remove_orphaned_tool_uses_all_removed() {
        use crate::kiro::model::requests::tool::ToolUseEntry;

        // Test tool_uses becomes None after removing all tool_uses
        let mut assistant_msg = AssistantMessage::new("I'll use a tool.");
        assistant_msg = assistant_msg.with_tool_uses(vec![
            ToolUseEntry::new("tool-1", "read").with_input(serde_json::json!({})),
        ]);

        let mut history = vec![
            Message::User(HistoryUserMessage::new("Do something", "claude-sonnet-4.5")),
            Message::Assistant(HistoryAssistantMessage {
                assistant_response_message: assistant_msg,
            }),
        ];

        let mut orphaned = std::collections::HashSet::new();
        orphaned.insert("tool-1".to_string());

        remove_orphaned_tool_uses(&mut history, &orphaned);

        // Verify tool_uses becomes None
        if let Message::Assistant(ref assistant_msg) = history[1] {
            assert!(
                assistant_msg.assistant_response_message.tool_uses.is_none(),
                "Should be None after removing all tool_uses"
            );
        } else {
            panic!("Should be Assistant message");
        }
    }
}
