//! Streaming response processing module
//!
//! Implements Kiro -> Anthropic streaming response conversion and SSE state management

use std::collections::HashMap;

use serde_json::json;
use uuid::Uuid;

use crate::kiro::model::events::Event;

/// Find the nearest valid UTF-8 character boundary less than or equal to the target position
///
/// UTF-8 characters can occupy 1-4 bytes, slicing directly at byte position may cut in the middle
/// of a multi-byte character causing panic. This function searches backward from the target position
/// to find the nearest valid character boundary.
fn find_char_boundary(s: &str, target: usize) -> usize {
    if target >= s.len() {
        return s.len();
    }
    if target == 0 {
        return 0;
    }
    // Search backward from target position for valid character boundary
    let mut pos = target;
    while pos > 0 && !s.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Quote characters to skip
///
/// When thinking tags are wrapped by these characters, they are considered as quoting the tag rather than actual tags:
/// - Backtick (`): inline code
/// - Double quote ("): string
/// - Single quote ('): string
const QUOTE_CHARS: &[u8] = &[
    b'`', b'"', b'\'', b'\\', b'#', b'!', b'@', b'$', b'%', b'^', b'&', b'*', b'(', b')', b'-',
    b'_', b'=', b'+', b'[', b']', b'{', b'}', b';', b':', b'<', b'>', b',', b'.', b'?', b'/',
];

/// Check if the character at the specified position is a quote character
fn is_quote_char(buffer: &str, pos: usize) -> bool {
    buffer
        .as_bytes()
        .get(pos)
        .map(|c| QUOTE_CHARS.contains(c))
        .unwrap_or(false)
}

/// Find the real thinking end tag (not wrapped by quote characters, and followed by double newline)
///
/// When the model mentions `</thinking>` during thinking, it's usually wrapped by backticks, quotes, etc.,
/// or has other content on the same line (like "about </thinking> tag").
/// This function skips these cases and only returns the position of the real end tag.
///
/// Skipped cases:
/// - Wrapped by quote characters (backticks, quotes, etc.)
/// - Not followed by double newline (real end tag is followed by `\n\n`)
/// - Tag at buffer end (need to wait for more content during streaming)
///
/// # Arguments
/// - `buffer`: The string to search
///
/// # Returns
/// - `Some(pos)`: Starting position of the real end tag
/// - `None`: No real end tag found
fn find_real_thinking_end_tag(buffer: &str) -> Option<usize> {
    const TAG: &str = "</thinking>";
    let mut search_start = 0;

    while let Some(pos) = buffer[search_start..].find(TAG) {
        let absolute_pos = search_start + pos;

        // Check if preceded by quote character
        let has_quote_before = absolute_pos > 0 && is_quote_char(buffer, absolute_pos - 1);

        // Check if followed by quote character
        let after_pos = absolute_pos + TAG.len();
        let has_quote_after = is_quote_char(buffer, after_pos);

        // If wrapped by quote characters, skip
        if has_quote_before || has_quote_after {
            search_start = absolute_pos + 1;
            continue;
        }

        // Check content after the tag
        let after_content = &buffer[after_pos..];

        // If content after tag is insufficient to determine if there's double newline, wait for more content
        if after_content.len() < 2 {
            return None;
        }

        // Real thinking end tag is followed by double newline `\n\n`
        if after_content.starts_with("\n\n") {
            return Some(absolute_pos);
        }

        // Not double newline, skip and continue searching
        search_start = absolute_pos + 1;
    }

    None
}

/// Find thinking end tag at buffer end (allowing only whitespace after it)
///
/// Used for "boundary event" scenarios: e.g., when thinking ends and immediately enters tool_use, or stream ends,
/// where `</thinking>` may not be followed by `\n\n`, but the end tag should still be recognized and filtered.
///
/// Constraint: Only consider it as end tag when everything after `</thinking>` is whitespace,
/// to avoid false positives when `</thinking>` is mentioned (not as end tag) within thinking content.
fn find_real_thinking_end_tag_at_buffer_end(buffer: &str) -> Option<usize> {
    const TAG: &str = "</thinking>";
    let mut search_start = 0;

    while let Some(pos) = buffer[search_start..].find(TAG) {
        let absolute_pos = search_start + pos;

        // Check if preceded by quote character
        let has_quote_before = absolute_pos > 0 && is_quote_char(buffer, absolute_pos - 1);

        // Check if followed by quote character
        let after_pos = absolute_pos + TAG.len();
        let has_quote_after = is_quote_char(buffer, after_pos);

        if has_quote_before || has_quote_after {
            search_start = absolute_pos + 1;
            continue;
        }

        // Only consider it as end tag when everything after is whitespace
        if buffer[after_pos..].trim().is_empty() {
            return Some(absolute_pos);
        }

        search_start = absolute_pos + 1;
    }

    None
}

/// Find the real thinking start tag (not wrapped by quote characters)
///
/// Similar to `find_real_thinking_end_tag`, skips start tags wrapped by quote characters.
fn find_real_thinking_start_tag(buffer: &str) -> Option<usize> {
    const TAG: &str = "<thinking>";
    let mut search_start = 0;

    while let Some(pos) = buffer[search_start..].find(TAG) {
        let absolute_pos = search_start + pos;

        // Check if preceded by quote character
        let has_quote_before = absolute_pos > 0 && is_quote_char(buffer, absolute_pos - 1);

        // Check if followed by quote character
        let after_pos = absolute_pos + TAG.len();
        let has_quote_after = is_quote_char(buffer, after_pos);

        // If not wrapped by quote characters, it's the real start tag
        if !has_quote_before && !has_quote_after {
            return Some(absolute_pos);
        }

        // Continue searching for next match
        search_start = absolute_pos + 1;
    }

    None
}

/// SSE event
#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event: String,
    pub data: serde_json::Value,
}

impl SseEvent {
    pub fn new(event: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            event: event.into(),
            data,
        }
    }

    /// Format as SSE string
    pub fn to_sse_string(&self) -> String {
        format!(
            "event: {}\ndata: {}\n\n",
            self.event,
            serde_json::to_string(&self.data).unwrap_or_default()
        )
    }
}

/// Content block state
#[derive(Debug, Clone)]
struct BlockState {
    block_type: String,
    started: bool,
    stopped: bool,
}

impl BlockState {
    fn new(block_type: impl Into<String>) -> Self {
        Self {
            block_type: block_type.into(),
            started: false,
            stopped: false,
        }
    }
}

/// SSE state manager
///
/// Ensures SSE event sequence conforms to Claude API specification:
/// 1. message_start can only appear once
/// 2. content_block must start, then delta, then stop
/// 3. message_delta can only appear once, and after all content_block_stop
/// 4. message_stop at the end
#[derive(Debug)]
pub struct SseStateManager {
    /// Whether message_start has been sent
    message_started: bool,
    /// Whether message_delta has been sent
    message_delta_sent: bool,
    /// Active content block states
    active_blocks: HashMap<i32, BlockState>,
    /// Whether message has ended
    message_ended: bool,
    /// Next block index
    next_block_index: i32,
    /// Current stop_reason
    stop_reason: Option<String>,
    /// Whether there are tool calls
    has_tool_use: bool,
}

impl Default for SseStateManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SseStateManager {
    pub fn new() -> Self {
        Self {
            message_started: false,
            message_delta_sent: false,
            active_blocks: HashMap::new(),
            message_ended: false,
            next_block_index: 0,
            stop_reason: None,
            has_tool_use: false,
        }
    }

    /// Check if the specified block is in an open state that can receive delta
    fn is_block_open_of_type(&self, index: i32, expected_type: &str) -> bool {
        self.active_blocks
            .get(&index)
            .is_some_and(|b| b.started && !b.stopped && b.block_type == expected_type)
    }

    /// Get next block index
    pub fn next_block_index(&mut self) -> i32 {
        let index = self.next_block_index;
        self.next_block_index += 1;
        index
    }

    /// Record tool call
    pub fn set_has_tool_use(&mut self, has: bool) {
        self.has_tool_use = has;
    }

    /// Set stop_reason
    pub fn set_stop_reason(&mut self, reason: impl Into<String>) {
        self.stop_reason = Some(reason.into());
    }

    /// Check if there are non-thinking type content blocks (like text or tool_use)
    fn has_non_thinking_blocks(&self) -> bool {
        self.active_blocks
            .values()
            .any(|b| b.block_type != "thinking")
    }

    /// Get final stop_reason
    pub fn get_stop_reason(&self) -> String {
        if let Some(ref reason) = self.stop_reason {
            reason.clone()
        } else if self.has_tool_use {
            "tool_use".to_string()
        } else {
            "end_turn".to_string()
        }
    }

    /// Handle message_start event
    pub fn handle_message_start(&mut self, event: serde_json::Value) -> Option<SseEvent> {
        if self.message_started {
            tracing::debug!("Skipping duplicate message_start event");
            return None;
        }
        self.message_started = true;
        Some(SseEvent::new("message_start", event))
    }

    /// Handle content_block_start event
    pub fn handle_content_block_start(
        &mut self,
        index: i32,
        block_type: &str,
        data: serde_json::Value,
    ) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // If it's a tool_use block, close previous text blocks first
        if block_type == "tool_use" {
            self.has_tool_use = true;
            for (block_index, block) in self.active_blocks.iter_mut() {
                if block.block_type == "text" && block.started && !block.stopped {
                    // Automatically send content_block_stop to close text block
                    events.push(SseEvent::new(
                        "content_block_stop",
                        json!({
                            "type": "content_block_stop",
                            "index": block_index
                        }),
                    ));
                    block.stopped = true;
                }
            }
        }

        // Check if block already exists
        if let Some(block) = self.active_blocks.get_mut(&index) {
            if block.started {
                tracing::debug!("Block {} already started, skipping duplicate content_block_start", index);
                return events;
            }
            block.started = true;
        } else {
            let mut block = BlockState::new(block_type);
            block.started = true;
            self.active_blocks.insert(index, block);
        }

        events.push(SseEvent::new("content_block_start", data));
        events
    }

    /// Handle content_block_delta event
    pub fn handle_content_block_delta(
        &mut self,
        index: i32,
        data: serde_json::Value,
    ) -> Option<SseEvent> {
        // Ensure block has started
        if let Some(block) = self.active_blocks.get(&index) {
            if !block.started || block.stopped {
                tracing::warn!(
                    "Block {} state abnormal: started={}, stopped={}",
                    index,
                    block.started,
                    block.stopped
                );
                return None;
            }
        } else {
            // Block doesn't exist, may need to create first
            tracing::warn!("Received delta event for unknown block {}", index);
            return None;
        }

        Some(SseEvent::new("content_block_delta", data))
    }

    /// Handle content_block_stop event
    pub fn handle_content_block_stop(&mut self, index: i32) -> Option<SseEvent> {
        if let Some(block) = self.active_blocks.get_mut(&index) {
            if block.stopped {
                tracing::debug!("Block {} already stopped, skipping duplicate content_block_stop", index);
                return None;
            }
            block.stopped = true;
            return Some(SseEvent::new(
                "content_block_stop",
                json!({
                    "type": "content_block_stop",
                    "index": index
                }),
            ));
        }
        None
    }

    /// Generate final event sequence
    pub fn generate_final_events(
        &mut self,
        input_tokens: i32,
        output_tokens: i32,
    ) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // Close all unclosed blocks
        for (index, block) in self.active_blocks.iter_mut() {
            if block.started && !block.stopped {
                events.push(SseEvent::new(
                    "content_block_stop",
                    json!({
                        "type": "content_block_stop",
                        "index": index
                    }),
                ));
                block.stopped = true;
            }
        }

        // Send message_delta
        if !self.message_delta_sent {
            self.message_delta_sent = true;
            events.push(SseEvent::new(
                "message_delta",
                json!({
                    "type": "message_delta",
                    "delta": {
                        "stop_reason": self.get_stop_reason(),
                        "stop_sequence": null
                    },
                    "usage": {
                        "input_tokens": input_tokens,
                        "output_tokens": output_tokens
                    }
                }),
            ));
        }

        // Send message_stop
        if !self.message_ended {
            self.message_ended = true;
            events.push(SseEvent::new(
                "message_stop",
                json!({ "type": "message_stop" }),
            ));
        }

        events
    }
}

/// Context window size (200k tokens)
const CONTEXT_WINDOW_SIZE: i32 = 200_000;

/// Stream processing context
pub struct StreamContext {
    /// SSE state manager
    pub state_manager: SseStateManager,
    /// Requested model name
    pub model: String,
    /// Message ID
    pub message_id: String,
    /// Input tokens (estimated value)
    pub input_tokens: i32,
    /// Actual input tokens calculated from contextUsageEvent
    pub context_input_tokens: Option<i32>,
    /// Output tokens accumulated
    pub output_tokens: i32,
    /// Tool block index mapping (tool_id -> block_index)
    pub tool_block_indices: HashMap<String, i32>,
    /// Whether thinking is enabled
    pub thinking_enabled: bool,
    /// Thinking content buffer
    pub thinking_buffer: String,
    /// Whether inside thinking block
    pub in_thinking_block: bool,
    /// Whether thinking block extraction is complete
    pub thinking_extracted: bool,
    /// Thinking block index
    pub thinking_block_index: Option<i32>,
    /// Text block index (dynamically allocated when thinking is enabled)
    pub text_block_index: Option<i32>,
    /// Whether to strip leading newline from thinking content
    /// When model outputs `<thinking>\n`, `\n` may be in the same chunk or next chunk as the tag
    strip_thinking_leading_newline: bool,
}

impl StreamContext {
    /// Create StreamContext with thinking enabled
    pub fn new_with_thinking(
        model: impl Into<String>,
        input_tokens: i32,
        thinking_enabled: bool,
    ) -> Self {
        Self {
            state_manager: SseStateManager::new(),
            model: model.into(),
            message_id: format!("msg_{}", Uuid::new_v4().to_string().replace('-', "")),
            input_tokens,
            context_input_tokens: None,
            output_tokens: 0,
            tool_block_indices: HashMap::new(),
            thinking_enabled,
            thinking_buffer: String::new(),
            in_thinking_block: false,
            thinking_extracted: false,
            thinking_block_index: None,
            text_block_index: None,
            strip_thinking_leading_newline: false,
        }
    }

    /// Generate message_start event
    pub fn create_message_start_event(&self) -> serde_json::Value {
        json!({
            "type": "message_start",
            "message": {
                "id": self.message_id,
                "type": "message",
                "role": "assistant",
                "content": [],
                "model": self.model,
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {
                    "input_tokens": self.input_tokens,
                    "output_tokens": 1
                }
            }
        })
    }

    /// Generate initial event sequence (message_start + text block start)
    ///
    /// When thinking is enabled, don't create text block at initialization, wait until actual content is received.
    /// This ensures thinking block (index 0) comes before text block (index 1).
    pub fn generate_initial_events(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // message_start
        let msg_start = self.create_message_start_event();
        if let Some(event) = self.state_manager.handle_message_start(msg_start) {
            events.push(event);
        }

        // If thinking is enabled, don't create text block here
        // thinking block and text block will be created in correct order in process_content_with_thinking
        if self.thinking_enabled {
            return events;
        }

        // Create initial text block (only when thinking is not enabled)
        let text_block_index = self.state_manager.next_block_index();
        self.text_block_index = Some(text_block_index);
        let text_block_events = self.state_manager.handle_content_block_start(
            text_block_index,
            "text",
            json!({
                "type": "content_block_start",
                "index": text_block_index,
                "content_block": {
                    "type": "text",
                    "text": ""
                }
            }),
        );
        events.extend(text_block_events);

        events
    }

    /// Process Kiro event and convert to Anthropic SSE events
    pub fn process_kiro_event(&mut self, event: &Event) -> Vec<SseEvent> {
        match event {
            Event::AssistantResponse(resp) => self.process_assistant_response(&resp.content),
            Event::ToolUse(tool_use) => self.process_tool_use(tool_use),
            Event::ContextUsage(context_usage) => {
                // Calculate actual input_tokens from context usage percentage
                // Formula: percentage * 200000 / 100 = percentage * 2000
                let actual_input_tokens = (context_usage.context_usage_percentage
                    * (CONTEXT_WINDOW_SIZE as f64)
                    / 100.0) as i32;
                self.context_input_tokens = Some(actual_input_tokens);
                // When context usage reaches 100%, set stop_reason to model_context_window_exceeded
                if context_usage.context_usage_percentage >= 100.0 {
                    self.state_manager
                        .set_stop_reason("model_context_window_exceeded");
                }
                tracing::debug!(
                    "Received contextUsageEvent: {}%, calculated input_tokens: {}",
                    context_usage.context_usage_percentage,
                    actual_input_tokens
                );
                Vec::new()
            }
            Event::Error {
                error_code,
                error_message,
            } => {
                tracing::error!("Received error event: {} - {}", error_code, error_message);
                Vec::new()
            }
            Event::Exception {
                exception_type,
                message,
            } => {
                // Handle ContentLengthExceededException
                if exception_type == "ContentLengthExceededException" {
                    self.state_manager.set_stop_reason("max_tokens");
                }
                tracing::warn!("Received exception event: {} - {}", exception_type, message);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    /// Process assistant response event
    fn process_assistant_response(&mut self, content: &str) -> Vec<SseEvent> {
        if content.is_empty() {
            return Vec::new();
        }

        // Estimate tokens
        self.output_tokens += estimate_tokens(content);

        // If thinking is enabled, need to process thinking block
        if self.thinking_enabled {
            return self.process_content_with_thinking(content);
        }

        // Non-thinking mode also reuses unified text_delta sending logic,
        // so that after tool_use auto-closes text block, it can self-heal and rebuild new text block, avoiding "swallowed characters".
        self.create_text_delta_events(content)
    }

    /// Process content containing thinking block
    fn process_content_with_thinking(&mut self, content: &str) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // Add content to buffer for processing
        self.thinking_buffer.push_str(content);

        loop {
            if !self.in_thinking_block && !self.thinking_extracted {
                // Find <thinking> start tag (skip those wrapped by backticks)
                if let Some(start_pos) = find_real_thinking_start_tag(&self.thinking_buffer) {
                    // Send content before <thinking> as text_delta
                    // Note: If preceding content is only whitespace (like \n\n returned by adaptive mode), skip it,
                    // to avoid creating meaningless text block before thinking block causing client parsing failure
                    let before_thinking = self.thinking_buffer[..start_pos].to_string();
                    if !before_thinking.is_empty() && !before_thinking.trim().is_empty() {
                        events.extend(self.create_text_delta_events(&before_thinking));
                    }

                    // Enter thinking block
                    self.in_thinking_block = true;
                    self.strip_thinking_leading_newline = true;
                    self.thinking_buffer =
                        self.thinking_buffer[start_pos + "<thinking>".len()..].to_string();

                    // Create thinking block content_block_start event
                    let thinking_index = self.state_manager.next_block_index();
                    self.thinking_block_index = Some(thinking_index);
                    let start_events = self.state_manager.handle_content_block_start(
                        thinking_index,
                        "thinking",
                        json!({
                            "type": "content_block_start",
                            "index": thinking_index,
                            "content_block": {
                                "type": "thinking",
                                "thinking": ""
                            }
                        }),
                    );
                    events.extend(start_events);
                } else {
                    // No <thinking> found, check if it might be partial tag
                    // Keep content that might be partial tag
                    let target_len = self
                        .thinking_buffer
                        .len()
                        .saturating_sub("<thinking>".len());
                    let safe_len = find_char_boundary(&self.thinking_buffer, target_len);
                    if safe_len > 0 {
                        let safe_content = self.thinking_buffer[..safe_len].to_string();
                        // If thinking hasn't been extracted yet, and safe content is only whitespace,
                        // don't send as text_delta, keep in buffer waiting for more content.
                        // This avoids the issue in 4.6 model where <thinking> tag splits across events,
                        // causing leading whitespace (like "\n\n") to be incorrectly created as text block,
                        // resulting in text block appearing before thinking block.
                        if !safe_content.is_empty() && !safe_content.trim().is_empty() {
                            events.extend(self.create_text_delta_events(&safe_content));
                            self.thinking_buffer = self.thinking_buffer[safe_len..].to_string();
                        }
                    }
                    break;
                }
            } else if self.in_thinking_block {
                // Strip newline immediately following <thinking> tag (may span chunks)
                if self.strip_thinking_leading_newline {
                    if self.thinking_buffer.starts_with('\n') {
                        self.thinking_buffer = self.thinking_buffer[1..].to_string();
                        self.strip_thinking_leading_newline = false;
                    } else if !self.thinking_buffer.is_empty() {
                        // buffer is not empty but doesn't start with \n, no longer need to strip
                        self.strip_thinking_leading_newline = false;
                    }
                    // Keep flag when buffer is empty, wait for next chunk
                }

                // Inside thinking block, find </thinking> end tag (skip those wrapped by backticks)
                if let Some(end_pos) = find_real_thinking_end_tag(&self.thinking_buffer) {
                    // Extract thinking content
                    let thinking_content = self.thinking_buffer[..end_pos].to_string();
                    if !thinking_content.is_empty() {
                        if let Some(thinking_index) = self.thinking_block_index {
                            events.push(
                                self.create_thinking_delta_event(thinking_index, &thinking_content),
                            );
                        }
                    }

                    // End thinking block
                    self.in_thinking_block = false;
                    self.thinking_extracted = true;

                    // Send empty thinking_delta event, then send content_block_stop event
                    if let Some(thinking_index) = self.thinking_block_index {
                        // First send empty thinking_delta
                        events.push(self.create_thinking_delta_event(thinking_index, ""));
                        // Then send content_block_stop
                        if let Some(stop_event) =
                            self.state_manager.handle_content_block_stop(thinking_index)
                        {
                            events.push(stop_event);
                        }
                    }

                    // Strip `</thinking>\n\n` (find_real_thinking_end_tag already confirmed \n\n exists)
                    self.thinking_buffer =
                        self.thinking_buffer[end_pos + "</thinking>\n\n".len()..].to_string();
                } else {
                    // No end tag found, send current buffer content as thinking_delta.
                    // Keep tail content that might be partial `</thinking>\n\n`:
                    // find_real_thinking_end_tag requires `\n\n` after tag to return Some,
                    // so reserved area must cover full length of `</thinking>\n\n` (13 bytes),
                    // otherwise when `</thinking>` is in buffer but `\n\n` hasn't arrived,
                    // first few characters of tag would be incorrectly sent as thinking_delta.
                    let target_len = self
                        .thinking_buffer
                        .len()
                        .saturating_sub("</thinking>\n\n".len());
                    let safe_len = find_char_boundary(&self.thinking_buffer, target_len);
                    if safe_len > 0 {
                        let safe_content = self.thinking_buffer[..safe_len].to_string();
                        if !safe_content.is_empty() {
                            if let Some(thinking_index) = self.thinking_block_index {
                                events.push(
                                    self.create_thinking_delta_event(thinking_index, &safe_content),
                                );
                            }
                        }
                        self.thinking_buffer = self.thinking_buffer[safe_len..].to_string();
                    }
                    break;
                }
            } else {
                // thinking extraction complete, remaining content as text_delta
                if !self.thinking_buffer.is_empty() {
                    let remaining = self.thinking_buffer.clone();
                    self.thinking_buffer.clear();
                    events.extend(self.create_text_delta_events(&remaining));
                }
                break;
            }
        }

        events
    }

    /// Create text_delta events
    ///
    /// If text block hasn't been created yet, will create text block first.
    /// When tool_use occurs, state machine will auto-close current text block; subsequent text will auto-create new text block to continue output.
    ///
    /// Return value includes possible content_block_start event and content_block_delta event.
    fn create_text_delta_events(&mut self, text: &str) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // If current text_block_index points to a block that has been closed (e.g., auto-stopped when tool_use starts),
        // discard that index and create new text block to continue output, avoiding delta being rejected by state machine causing "swallowed characters".
        if let Some(idx) = self.text_block_index {
            if !self.state_manager.is_block_open_of_type(idx, "text") {
                self.text_block_index = None;
            }
        }

        // Get or create text block index
        let text_index = if let Some(idx) = self.text_block_index {
            idx
        } else {
            // Text block not created yet, need to create first
            let idx = self.state_manager.next_block_index();
            self.text_block_index = Some(idx);

            // Send content_block_start event
            let start_events = self.state_manager.handle_content_block_start(
                idx,
                "text",
                json!({
                    "type": "content_block_start",
                    "index": idx,
                    "content_block": {
                        "type": "text",
                        "text": ""
                    }
                }),
            );
            events.extend(start_events);
            idx
        };

        // Send content_block_delta event
        if let Some(delta_event) = self.state_manager.handle_content_block_delta(
            text_index,
            json!({
                "type": "content_block_delta",
                "index": text_index,
                "delta": {
                    "type": "text_delta",
                    "text": text
                }
            }),
        ) {
            events.push(delta_event);
        }

        events
    }

    /// Create thinking_delta event
    fn create_thinking_delta_event(&self, index: i32, thinking: &str) -> SseEvent {
        SseEvent::new(
            "content_block_delta",
            json!({
                "type": "content_block_delta",
                "index": index,
                "delta": {
                    "type": "thinking_delta",
                    "thinking": thinking
                }
            }),
        )
    }

    /// Process tool use event
    fn process_tool_use(
        &mut self,
        tool_use: &crate::kiro::model::events::ToolUseEvent,
    ) -> Vec<SseEvent> {
        let mut events = Vec::new();

        self.state_manager.set_has_tool_use(true);

        // tool_use must occur after thinking ends.
        // But when `</thinking>` is not followed by `\n\n` (e.g., immediately followed by tool_use or stream ends),
        // thinking end tag will remain in thinking_buffer, causing subsequent flush to output `</thinking>` as content.
        // Here we do a "boundary scenario" end tag recognition and filtering before starting tool_use block.
        if self.thinking_enabled && self.in_thinking_block {
            if let Some(end_pos) = find_real_thinking_end_tag_at_buffer_end(&self.thinking_buffer) {
                let thinking_content = self.thinking_buffer[..end_pos].to_string();
                if !thinking_content.is_empty() {
                    if let Some(thinking_index) = self.thinking_block_index {
                        events.push(
                            self.create_thinking_delta_event(thinking_index, &thinking_content),
                        );
                    }
                }

                // End thinking block
                self.in_thinking_block = false;
                self.thinking_extracted = true;

                if let Some(thinking_index) = self.thinking_block_index {
                    // First send empty thinking_delta
                    events.push(self.create_thinking_delta_event(thinking_index, ""));
                    // Then send content_block_stop
                    if let Some(stop_event) =
                        self.state_manager.handle_content_block_stop(thinking_index)
                    {
                        events.push(stop_event);
                    }
                }

                // Treat content after end tag as regular text (usually empty or whitespace)
                let after_pos = end_pos + "</thinking>".len();
                let remaining = self.thinking_buffer[after_pos..].trim_start().to_string();
                self.thinking_buffer.clear();
                if !remaining.is_empty() {
                    events.extend(self.create_text_delta_events(&remaining));
                }
            }
        }

        // In thinking mode, process_content_with_thinking may temporarily store a small tail text to detect `<thinking>`.
        // If tool_use starts directly at this point, state machine will auto-close text block, causing this "pending output text" to appear swallowed by tool_use.
        // Constraint: Only flush buffer as regular text when not yet entered thinking block and thinking hasn't been extracted.
        if self.thinking_enabled
            && !self.in_thinking_block
            && !self.thinking_extracted
            && !self.thinking_buffer.is_empty()
        {
            let buffered = std::mem::take(&mut self.thinking_buffer);
            events.extend(self.create_text_delta_events(&buffered));
        }

        // Get or allocate block index
        let block_index = if let Some(&idx) = self.tool_block_indices.get(&tool_use.tool_use_id) {
            idx
        } else {
            let idx = self.state_manager.next_block_index();
            self.tool_block_indices
                .insert(tool_use.tool_use_id.clone(), idx);
            idx
        };

        // Send content_block_start
        let start_events = self.state_manager.handle_content_block_start(
            block_index,
            "tool_use",
            json!({
                "type": "content_block_start",
                "index": block_index,
                "content_block": {
                    "type": "tool_use",
                    "id": tool_use.tool_use_id,
                    "name": tool_use.name,
                    "input": {}
                }
            }),
        );
        events.extend(start_events);

        // Send parameter delta (ToolUseEvent.input is String type)
        if !tool_use.input.is_empty() {
            self.output_tokens += (tool_use.input.len() as i32 + 3) / 4; // Estimate token

            if let Some(delta_event) = self.state_manager.handle_content_block_delta(
                block_index,
                json!({
                    "type": "content_block_delta",
                    "index": block_index,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": tool_use.input
                    }
                }),
            ) {
                events.push(delta_event);
            }
        }

        // If this is a complete tool call (stop=true), send content_block_stop
        if tool_use.stop {
            if let Some(stop_event) = self.state_manager.handle_content_block_stop(block_index) {
                events.push(stop_event);
            }
        }

        events
    }

    /// Generate final event sequence
    pub fn generate_final_events(&mut self) -> Vec<SseEvent> {
        let mut events = Vec::new();

        // Flush remaining content in thinking_buffer
        if self.thinking_enabled && !self.thinking_buffer.is_empty() {
            if self.in_thinking_block {
                // End may have residual `</thinking>` (e.g., immediately followed by tool_use or stream ends), need to filter out end tag during flush.
                if let Some(end_pos) =
                    find_real_thinking_end_tag_at_buffer_end(&self.thinking_buffer)
                {
                    let thinking_content = self.thinking_buffer[..end_pos].to_string();
                    if !thinking_content.is_empty() {
                        if let Some(thinking_index) = self.thinking_block_index {
                            events.push(
                                self.create_thinking_delta_event(thinking_index, &thinking_content),
                            );
                        }
                    }

                    // Close thinking block: first send empty thinking_delta, then send content_block_stop
                    if let Some(thinking_index) = self.thinking_block_index {
                        events.push(self.create_thinking_delta_event(thinking_index, ""));
                        if let Some(stop_event) =
                            self.state_manager.handle_content_block_stop(thinking_index)
                        {
                            events.push(stop_event);
                        }
                    }

                    // Treat content after end tag as regular text (usually empty or whitespace)
                    let after_pos = end_pos + "</thinking>".len();
                    let remaining = self.thinking_buffer[after_pos..].trim_start().to_string();
                    self.thinking_buffer.clear();
                    self.in_thinking_block = false;
                    self.thinking_extracted = true;
                    if !remaining.is_empty() {
                        events.extend(self.create_text_delta_events(&remaining));
                    }
                } else {
                    // If still inside thinking block, send remaining content as thinking_delta
                    if let Some(thinking_index) = self.thinking_block_index {
                        events.push(
                            self.create_thinking_delta_event(thinking_index, &self.thinking_buffer),
                        );
                    }
                    // Close thinking block: first send empty thinking_delta, then send content_block_stop
                    if let Some(thinking_index) = self.thinking_block_index {
                        // First send empty thinking_delta
                        events.push(self.create_thinking_delta_event(thinking_index, ""));
                        // Then send content_block_stop
                        if let Some(stop_event) =
                            self.state_manager.handle_content_block_stop(thinking_index)
                        {
                            events.push(stop_event);
                        }
                    }
                }
            } else {
                // Otherwise send remaining content as text_delta
                let buffer_content = self.thinking_buffer.clone();
                events.extend(self.create_text_delta_events(&buffer_content));
            }
            self.thinking_buffer.clear();
        }

        // If entire stream only produced thinking block, no text and no tool_use,
        // set stop_reason to max_tokens (indicating model exhausted token budget on thinking),
        // and emit a complete set of text events (content is a single space), ensuring content array has text block
        if self.thinking_enabled
            && self.thinking_block_index.is_some()
            && !self.state_manager.has_non_thinking_blocks()
        {
            self.state_manager.set_stop_reason("max_tokens");
            events.extend(self.create_text_delta_events(" "));
        }

        // Use input_tokens calculated from contextUsageEvent, fallback to estimate if not available
        let final_input_tokens = self.context_input_tokens.unwrap_or(self.input_tokens);

        // Generate final events
        events.extend(
            self.state_manager
                .generate_final_events(final_input_tokens, self.output_tokens),
        );
        events
    }
}

/// Buffered stream processing context - for /cc/v1/messages streaming requests
///
/// Unlike `StreamContext`, this context buffers all events until stream ends,
/// then corrects `message_start` event with correct `input_tokens` calculated from `contextUsageEvent`.
///
/// Workflow:
/// 1. Use `StreamContext` to process all Kiro events normally
/// 2. Cache generated SSE events (instead of sending immediately)
/// 3. When stream ends, find `message_start` event and update its `input_tokens`
/// 4. Return all events at once
pub struct BufferedStreamContext {
    /// Internal stream processing context (reuses existing event processing logic)
    inner: StreamContext,
    /// All buffered events (including message_start, content_block_start, etc.)
    event_buffer: Vec<SseEvent>,
    /// Estimated input_tokens (for fallback)
    estimated_input_tokens: i32,
    /// Whether initial events have been generated
    initial_events_generated: bool,
}

impl BufferedStreamContext {
    /// Create buffered stream context
    pub fn new(
        model: impl Into<String>,
        estimated_input_tokens: i32,
        thinking_enabled: bool,
    ) -> Self {
        let inner =
            StreamContext::new_with_thinking(model, estimated_input_tokens, thinking_enabled);
        Self {
            inner,
            event_buffer: Vec::new(),
            estimated_input_tokens,
            initial_events_generated: false,
        }
    }

    /// Process Kiro event and buffer results
    ///
    /// Reuses StreamContext's event processing logic, but caches results instead of sending immediately.
    pub fn process_and_buffer(&mut self, event: &crate::kiro::model::events::Event) {
        // On first event processing, generate initial events (message_start, etc.) first
        if !self.initial_events_generated {
            let initial_events = self.inner.generate_initial_events();
            self.event_buffer.extend(initial_events);
            self.initial_events_generated = true;
        }

        // Process event and buffer results
        let events = self.inner.process_kiro_event(event);
        self.event_buffer.extend(events);
    }

    /// Finish stream processing and return all events
    ///
    /// This method will:
    /// 1. Generate final events (message_delta, message_stop)
    /// 2. Correct message_start event with correct input_tokens
    /// 3. Return all buffered events
    pub fn finish_and_get_all_events(&mut self) -> Vec<SseEvent> {
        // If no events were ever processed, still generate initial events
        if !self.initial_events_generated {
            let initial_events = self.inner.generate_initial_events();
            self.event_buffer.extend(initial_events);
            self.initial_events_generated = true;
        }

        // Generate final events
        let final_events = self.inner.generate_final_events();
        self.event_buffer.extend(final_events);

        // Get correct input_tokens
        let final_input_tokens = self
            .inner
            .context_input_tokens
            .unwrap_or(self.estimated_input_tokens);

        // Correct input_tokens in message_start event
        for event in &mut self.event_buffer {
            if event.event == "message_start" {
                if let Some(message) = event.data.get_mut("message") {
                    if let Some(usage) = message.get_mut("usage") {
                        usage["input_tokens"] = serde_json::json!(final_input_tokens);
                    }
                }
            }
        }

        std::mem::take(&mut self.event_buffer)
    }
}

/// Simple token estimation
fn estimate_tokens(text: &str) -> i32 {
    let chars: Vec<char> = text.chars().collect();
    let mut chinese_count = 0;
    let mut other_count = 0;

    for c in &chars {
        if *c >= '\u{4E00}' && *c <= '\u{9FFF}' {
            chinese_count += 1;
        } else {
            other_count += 1;
        }
    }

    // Chinese approx 1.5 chars/token, English approx 4 chars/token
    let chinese_tokens = (chinese_count * 2 + 2) / 3;
    let other_tokens = (other_count + 3) / 4;

    (chinese_tokens + other_tokens).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sse_event_format() {
        let event = SseEvent::new("message_start", json!({"type": "message_start"}));
        let sse_str = event.to_sse_string();

        assert!(sse_str.starts_with("event: message_start\n"));
        assert!(sse_str.contains("data: "));
        assert!(sse_str.ends_with("\n\n"));
    }

    #[test]
    fn test_sse_state_manager_message_start() {
        let mut manager = SseStateManager::new();

        // 第一次应该成功
        let event = manager.handle_message_start(json!({"type": "message_start"}));
        assert!(event.is_some());

        // 第二次应该被跳过
        let event = manager.handle_message_start(json!({"type": "message_start"}));
        assert!(event.is_none());
    }

    #[test]
    fn test_sse_state_manager_block_lifecycle() {
        let mut manager = SseStateManager::new();

        // 创建块
        let events = manager.handle_content_block_start(0, "text", json!({}));
        assert_eq!(events.len(), 1);

        // delta
        let event = manager.handle_content_block_delta(0, json!({}));
        assert!(event.is_some());

        // stop
        let event = manager.handle_content_block_stop(0);
        assert!(event.is_some());

        // 重复 stop 应该被跳过
        let event = manager.handle_content_block_stop(0);
        assert!(event.is_none());
    }

    #[test]
    fn test_text_delta_after_tool_use_restarts_text_block() {
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, false);

        let initial_events = ctx.generate_initial_events();
        assert!(
            initial_events
                .iter()
                .any(|e| e.event == "content_block_start"
                    && e.data["content_block"]["type"] == "text")
        );

        let initial_text_index = ctx
            .text_block_index
            .expect("initial text block index should exist");

        // tool_use 开始会自动关闭现有 text block
        let tool_events = ctx.process_tool_use(&crate::kiro::model::events::ToolUseEvent {
            name: "test_tool".to_string(),
            tool_use_id: "tool_1".to_string(),
            input: "{}".to_string(),
            stop: false,
        });
        assert!(
            tool_events.iter().any(|e| {
                e.event == "content_block_stop"
                    && e.data["index"].as_i64() == Some(initial_text_index as i64)
            }),
            "tool_use should stop the previous text block"
        );

        // 之后再来文本增量，应自动创建新的 text block 而不是往已 stop 的块里写 delta
        let text_events = ctx.process_assistant_response("hello");
        let new_text_start_index = text_events.iter().find_map(|e| {
            if e.event == "content_block_start" && e.data["content_block"]["type"] == "text" {
                e.data["index"].as_i64()
            } else {
                None
            }
        });
        assert!(
            new_text_start_index.is_some(),
            "should start a new text block"
        );
        assert_ne!(
            new_text_start_index.unwrap(),
            initial_text_index as i64,
            "new text block index should differ from the stopped one"
        );
        assert!(
            text_events.iter().any(|e| {
                e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "text_delta"
                    && e.data["delta"]["text"] == "hello"
            }),
            "should emit text_delta after restarting text block"
        );
    }

    #[test]
    fn test_tool_use_flushes_pending_thinking_buffer_text_before_tool_block() {
        // In thinking mode, short text may be temporarily stored in thinking_buffer to wait for cross-chunk `<thinking>` matching.
        // When tool_use immediately follows, should flush this text first, then start tool_use block.
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        // Two short texts (2 Chinese characters each), total length may still not satisfy safe_len>0 output condition,
        // so they will remain in thinking_buffer waiting for subsequent chunks.
        let ev1 = ctx.process_assistant_response("有修");
        assert!(
            ev1.iter().all(|e| e.event != "content_block_delta"),
            "short prefix should be buffered under thinking mode"
        );
        let ev2 = ctx.process_assistant_response("改：");
        assert!(
            ev2.iter().all(|e| e.event != "content_block_delta"),
            "short prefix should still be buffered under thinking mode"
        );

        let events = ctx.process_tool_use(&crate::kiro::model::events::ToolUseEvent {
            name: "Write".to_string(),
            tool_use_id: "tool_1".to_string(),
            input: "{}".to_string(),
            stop: false,
        });

        let text_start_index = events.iter().find_map(|e| {
            if e.event == "content_block_start" && e.data["content_block"]["type"] == "text" {
                e.data["index"].as_i64()
            } else {
                None
            }
        });
        let pos_text_delta = events.iter().position(|e| {
            e.event == "content_block_delta" && e.data["delta"]["type"] == "text_delta"
        });
        let pos_text_stop = text_start_index.and_then(|idx| {
            events.iter().position(|e| {
                e.event == "content_block_stop" && e.data["index"].as_i64() == Some(idx)
            })
        });
        let pos_tool_start = events.iter().position(|e| {
            e.event == "content_block_start" && e.data["content_block"]["type"] == "tool_use"
        });

        assert!(
            text_start_index.is_some(),
            "should start a text block to flush buffered text"
        );
        assert!(
            pos_text_delta.is_some(),
            "should flush buffered text as text_delta"
        );
        assert!(
            pos_text_stop.is_some(),
            "should stop text block before tool_use block starts"
        );
        assert!(pos_tool_start.is_some(), "should start tool_use block");

        let pos_text_delta = pos_text_delta.unwrap();
        let pos_text_stop = pos_text_stop.unwrap();
        let pos_tool_start = pos_tool_start.unwrap();

        assert!(
            pos_text_delta < pos_text_stop && pos_text_stop < pos_tool_start,
            "ordering should be: text_delta -> text_stop -> tool_use_start"
        );

        assert!(
            events.iter().any(|e| {
                e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "text_delta"
                    && e.data["delta"]["text"] == "有修改："
            }),
            "flushed text should equal the buffered prefix"
        );
    }

    #[test]
    fn test_estimate_tokens() {
        assert!(estimate_tokens("Hello") > 0);
        assert!(estimate_tokens("你好") > 0);
        assert!(estimate_tokens("Hello 你好") > 0);
    }

    #[test]
    fn test_find_real_thinking_start_tag_basic() {
        // Basic case: normal start tag
        assert_eq!(find_real_thinking_start_tag("<thinking>"), Some(0));
        assert_eq!(find_real_thinking_start_tag("prefix<thinking>"), Some(6));
    }

    #[test]
    fn test_find_real_thinking_start_tag_with_backticks() {
        // Wrapped by backticks should be skipped
        assert_eq!(find_real_thinking_start_tag("`<thinking>`"), None);
        assert_eq!(find_real_thinking_start_tag("use `<thinking>` tag"), None);

        // First has wrapped one, then has real start tag
        assert_eq!(
            find_real_thinking_start_tag("about `<thinking>` tag<thinking>content"),
            Some(22)
        );
    }

    #[test]
    fn test_find_real_thinking_start_tag_with_quotes() {
        // Wrapped by double quotes should be skipped
        assert_eq!(find_real_thinking_start_tag("\"<thinking>\""), None);
        assert_eq!(find_real_thinking_start_tag("the \"<thinking>\" tag"), None);

        // Wrapped by single quotes should be skipped
        assert_eq!(find_real_thinking_start_tag("'<thinking>'"), None);

        // Mixed case
        assert_eq!(
            find_real_thinking_start_tag("about \"<thinking>\" and '<thinking>' then<thinking>"),
            Some(40)
        );
    }

    #[test]
    fn test_find_real_thinking_end_tag_basic() {
        // Basic case: normal end tag followed by double newline
        assert_eq!(find_real_thinking_end_tag("</thinking>\n\n"), Some(0));
        assert_eq!(
            find_real_thinking_end_tag("content</thinking>\n\n"),
            Some(7)
        );
        assert_eq!(
            find_real_thinking_end_tag("some text</thinking>\n\nmore text"),
            Some(9)
        );

        // Case without double newline
        assert_eq!(find_real_thinking_end_tag("</thinking>"), None);
        assert_eq!(find_real_thinking_end_tag("</thinking>\n"), None);
        assert_eq!(find_real_thinking_end_tag("</thinking> more"), None);
    }

    #[test]
    fn test_find_real_thinking_end_tag_with_backticks() {
        // Wrapped by backticks should be skipped
        assert_eq!(find_real_thinking_end_tag("`</thinking>`\n\n"), None);
        assert_eq!(
            find_real_thinking_end_tag("mention `</thinking>` in code\n\n"),
            None
        );

        // Only has backtick before
        assert_eq!(find_real_thinking_end_tag("`</thinking>\n\n"), None);

        // Only has backtick after
        assert_eq!(find_real_thinking_end_tag("</thinking>`\n\n"), None);
    }

    #[test]
    fn test_find_real_thinking_end_tag_with_quotes() {
        // Wrapped by double quotes should be skipped
        assert_eq!(find_real_thinking_end_tag("\"</thinking>\"\n\n"), None);
        assert_eq!(
            find_real_thinking_end_tag("the string \"</thinking>\" is a tag\n\n"),
            None
        );

        // Wrapped by single quotes should be skipped
        assert_eq!(find_real_thinking_end_tag("'</thinking>'\n\n"), None);
        assert_eq!(
            find_real_thinking_end_tag("use '</thinking>' as marker\n\n"),
            None
        );

        // Mixed case: double quote wrapped then has real tag
        assert_eq!(
            find_real_thinking_end_tag("about \"</thinking>\" tag</thinking>\n\n"),
            Some(23)
        );

        // Mixed case: single quote wrapped then has real tag
        assert_eq!(
            find_real_thinking_end_tag("about '</thinking>' tag</thinking>\n\n"),
            Some(23)
        );
    }

    #[test]
    fn test_find_real_thinking_end_tag_mixed() {
        // First has wrapped one, then has real end tag
        assert_eq!(
            find_real_thinking_end_tag("discussing `</thinking>` tag</thinking>\n\n"),
            Some(28)
        );

        // Multiple wrapped ones, last one is real
        assert_eq!(
            find_real_thinking_end_tag("`</thinking>` and `</thinking>` done</thinking>\n\n"),
            Some(36)
        );

        // Multiple quote character types mixed
        assert_eq!(
            find_real_thinking_end_tag(
                "`</thinking>` and \"</thinking>\" and '</thinking>' done</thinking>\n\n"
            ),
            Some(54)
        );
    }

    #[test]
    fn test_tool_use_immediately_after_thinking_filters_end_tag_and_closes_thinking_block() {
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let mut all_events = Vec::new();

        // thinking content ends with `</thinking>`, but not followed by `\n\n` (simulating scenario immediately followed by tool_use)
        all_events.extend(ctx.process_assistant_response("<thinking>abc</thinking>"));

        let tool_events = ctx.process_tool_use(&crate::kiro::model::events::ToolUseEvent {
            name: "Write".to_string(),
            tool_use_id: "tool_1".to_string(),
            input: "{}".to_string(),
            stop: false,
        });
        all_events.extend(tool_events);

        all_events.extend(ctx.generate_final_events());

        // Should not output `</thinking>` as thinking content
        assert!(
            all_events.iter().all(|e| {
                !(e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "thinking_delta"
                    && e.data["delta"]["thinking"] == "</thinking>")
            }),
            "`</thinking>` should be filtered from output"
        );

        // thinking block must be closed before tool_use block
        let thinking_index = ctx
            .thinking_block_index
            .expect("thinking block index should exist");
        let pos_thinking_stop = all_events.iter().position(|e| {
            e.event == "content_block_stop"
                && e.data["index"].as_i64() == Some(thinking_index as i64)
        });
        let pos_tool_start = all_events.iter().position(|e| {
            e.event == "content_block_start" && e.data["content_block"]["type"] == "tool_use"
        });
        assert!(
            pos_thinking_stop.is_some(),
            "thinking block should be stopped"
        );
        assert!(pos_tool_start.is_some(), "tool_use block should be started");
        assert!(
            pos_thinking_stop.unwrap() < pos_tool_start.unwrap(),
            "thinking block should stop before tool_use block starts"
        );
    }

    #[test]
    fn test_final_flush_filters_standalone_thinking_end_tag() {
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let mut all_events = Vec::new();
        all_events.extend(ctx.process_assistant_response("<thinking>abc</thinking>"));
        all_events.extend(ctx.generate_final_events());

        assert!(
            all_events.iter().all(|e| {
                !(e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "thinking_delta"
                    && e.data["delta"]["thinking"] == "</thinking>")
            }),
            "`</thinking>` should be filtered during final flush"
        );
    }

    #[test]
    fn test_thinking_strips_leading_newline_same_chunk() {
        // <thinking>\n in same chunk, \n should be stripped
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let events = ctx.process_assistant_response("<thinking>\nHello world");

        // Find all thinking_delta events
        let thinking_deltas: Vec<_> = events
            .iter()
            .filter(|e| {
                e.event == "content_block_delta" && e.data["delta"]["type"] == "thinking_delta"
            })
            .collect();

        // Concatenate all thinking content
        let full_thinking: String = thinking_deltas
            .iter()
            .map(|e| e.data["delta"]["thinking"].as_str().unwrap_or(""))
            .collect();

        assert!(
            !full_thinking.starts_with('\n'),
            "thinking content should not start with \\n, got: {:?}",
            full_thinking
        );
    }

    #[test]
    fn test_thinking_strips_leading_newline_cross_chunk() {
        // <thinking> at end of first chunk, \n at start of second chunk
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let events1 = ctx.process_assistant_response("<thinking>");
        let events2 = ctx.process_assistant_response("\nHello world");

        let mut all_events = Vec::new();
        all_events.extend(events1);
        all_events.extend(events2);

        let thinking_deltas: Vec<_> = all_events
            .iter()
            .filter(|e| {
                e.event == "content_block_delta" && e.data["delta"]["type"] == "thinking_delta"
            })
            .collect();

        let full_thinking: String = thinking_deltas
            .iter()
            .map(|e| e.data["delta"]["thinking"].as_str().unwrap_or(""))
            .collect();

        assert!(
            !full_thinking.starts_with('\n'),
            "thinking content should not start with \\n across chunks, got: {:?}",
            full_thinking
        );
    }

    #[test]
    fn test_thinking_no_strip_when_no_leading_newline() {
        // <thinking> directly followed by content (no \n), content should be fully preserved
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let events = ctx.process_assistant_response("<thinking>abc</thinking>\n\ntext");

        let thinking_deltas: Vec<_> = events
            .iter()
            .filter(|e| {
                e.event == "content_block_delta" && e.data["delta"]["type"] == "thinking_delta"
            })
            .collect();

        let full_thinking: String = thinking_deltas
            .iter()
            .filter(|e| !e.data["delta"]["thinking"].as_str().unwrap_or("").is_empty())
            .map(|e| e.data["delta"]["thinking"].as_str().unwrap_or(""))
            .collect();

        assert_eq!(full_thinking, "abc", "thinking content should be 'abc'");
    }

    #[test]
    fn test_text_after_thinking_strips_leading_newlines() {
        // Text after `</thinking>\n\n` should not start with \n\n
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let events =
            ctx.process_assistant_response("<thinking>\nabc</thinking>\n\n你好");

        let text_deltas: Vec<_> = events
            .iter()
            .filter(|e| {
                e.event == "content_block_delta" && e.data["delta"]["type"] == "text_delta"
            })
            .collect();

        let full_text: String = text_deltas
            .iter()
            .map(|e| e.data["delta"]["text"].as_str().unwrap_or(""))
            .collect();

        assert!(
            !full_text.starts_with('\n'),
            "text after thinking should not start with \\n, got: {:?}",
            full_text
        );
        assert_eq!(full_text, "你好");
    }

    /// Helper function: extract concatenated content of all thinking_delta from event list
    fn collect_thinking_content(events: &[SseEvent]) -> String {
        events
            .iter()
            .filter(|e| {
                e.event == "content_block_delta" && e.data["delta"]["type"] == "thinking_delta"
            })
            .map(|e| e.data["delta"]["thinking"].as_str().unwrap_or(""))
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// Helper function: extract concatenated content of all text_delta from event list
    fn collect_text_content(events: &[SseEvent]) -> String {
        events
            .iter()
            .filter(|e| {
                e.event == "content_block_delta" && e.data["delta"]["type"] == "text_delta"
            })
            .map(|e| e.data["delta"]["text"].as_str().unwrap_or(""))
            .collect()
    }

    #[test]
    fn test_end_tag_newlines_split_across_events() {
        // `</thinking>\n` in chunk 1, `\n` in chunk 2, `text` in chunk 3
        // Ensure `</thinking>` is not partially sent as thinking content
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let mut all = Vec::new();
        all.extend(ctx.process_assistant_response("<thinking>\nabc</thinking>\n"));
        all.extend(ctx.process_assistant_response("\n"));
        all.extend(ctx.process_assistant_response("你好"));
        all.extend(ctx.generate_final_events());

        let thinking = collect_thinking_content(&all);
        assert_eq!(thinking, "abc", "thinking should be 'abc', got: {:?}", thinking);

        let text = collect_text_content(&all);
        assert_eq!(text, "你好", "text should be '你好', got: {:?}", text);
    }

    #[test]
    fn test_end_tag_alone_in_chunk_then_newlines_in_next() {
        // `</thinking>` alone in one chunk, `\n\ntext` in next chunk
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let mut all = Vec::new();
        all.extend(ctx.process_assistant_response("<thinking>\nabc</thinking>"));
        all.extend(ctx.process_assistant_response("\n\n你好"));
        all.extend(ctx.generate_final_events());

        let thinking = collect_thinking_content(&all);
        assert_eq!(thinking, "abc", "thinking should be 'abc', got: {:?}", thinking);

        let text = collect_text_content(&all);
        assert_eq!(text, "你好", "text should be '你好', got: {:?}", text);
    }

    #[test]
    fn test_start_tag_newline_split_across_events() {
        // `\n\n` in chunk 1, `<thinking>` in chunk 2, `\n` in chunk 3
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let mut all = Vec::new();
        all.extend(ctx.process_assistant_response("\n\n"));
        all.extend(ctx.process_assistant_response("<thinking>"));
        all.extend(ctx.process_assistant_response("\n"));
        all.extend(ctx.process_assistant_response("abc</thinking>\n\ntext"));
        all.extend(ctx.generate_final_events());

        let thinking = collect_thinking_content(&all);
        assert_eq!(thinking, "abc", "thinking should be 'abc', got: {:?}", thinking);

        let text = collect_text_content(&all);
        assert_eq!(text, "text", "text should be 'text', got: {:?}", text);
    }

    #[test]
    fn test_full_flow_maximally_split() {
        // Extreme split: each key boundary in different chunk
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let mut all = Vec::new();
        // \n\n<thinking>\n split into multiple segments
        all.extend(ctx.process_assistant_response("\n"));
        all.extend(ctx.process_assistant_response("\n"));
        all.extend(ctx.process_assistant_response("<thin"));
        all.extend(ctx.process_assistant_response("king>"));
        all.extend(ctx.process_assistant_response("\n"));
        all.extend(ctx.process_assistant_response("hello"));
        // </thinking>\n\n split into multiple segments
        all.extend(ctx.process_assistant_response("</thi"));
        all.extend(ctx.process_assistant_response("nking>"));
        all.extend(ctx.process_assistant_response("\n"));
        all.extend(ctx.process_assistant_response("\n"));
        all.extend(ctx.process_assistant_response("world"));
        all.extend(ctx.generate_final_events());

        let thinking = collect_thinking_content(&all);
        assert_eq!(thinking, "hello", "thinking should be 'hello', got: {:?}", thinking);

        let text = collect_text_content(&all);
        assert_eq!(text, "world", "text should be 'world', got: {:?}", text);
    }

    #[test]
    fn test_thinking_only_sets_max_tokens_stop_reason() {
        // Entire stream only has thinking block, no text and no tool_use, stop_reason should be max_tokens
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let mut all_events = Vec::new();
        all_events.extend(ctx.process_assistant_response("<thinking>\nabc</thinking>"));
        all_events.extend(ctx.generate_final_events());

        let message_delta = all_events
            .iter()
            .find(|e| e.event == "message_delta")
            .expect("should have message_delta event");

        assert_eq!(
            message_delta.data["delta"]["stop_reason"], "max_tokens",
            "stop_reason should be max_tokens when only thinking is produced"
        );

        // Should emit a complete set of text events (content_block_start + delta space + content_block_stop)
        assert!(
            all_events.iter().any(|e| {
                e.event == "content_block_start" && e.data["content_block"]["type"] == "text"
            }),
            "should emit text content_block_start"
        );
        assert!(
            all_events.iter().any(|e| {
                e.event == "content_block_delta"
                    && e.data["delta"]["type"] == "text_delta"
                    && e.data["delta"]["text"] == " "
            }),
            "should emit text_delta with a single space"
        );
        // text block should be auto-closed by generate_final_events
        let text_block_index = all_events
            .iter()
            .find_map(|e| {
                if e.event == "content_block_start" && e.data["content_block"]["type"] == "text" {
                    e.data["index"].as_i64()
                } else {
                    None
                }
            })
            .expect("text block should exist");
        assert!(
            all_events.iter().any(|e| {
                e.event == "content_block_stop"
                    && e.data["index"].as_i64() == Some(text_block_index)
            }),
            "text block should be stopped"
        );
    }

    #[test]
    fn test_thinking_with_text_keeps_end_turn_stop_reason() {
        // thinking + text case, stop_reason should be end_turn
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let mut all_events = Vec::new();
        all_events.extend(ctx.process_assistant_response("<thinking>\nabc</thinking>\n\nHello"));
        all_events.extend(ctx.generate_final_events());

        let message_delta = all_events
            .iter()
            .find(|e| e.event == "message_delta")
            .expect("should have message_delta event");

        assert_eq!(
            message_delta.data["delta"]["stop_reason"], "end_turn",
            "stop_reason should be end_turn when text is also produced"
        );
    }

    #[test]
    fn test_thinking_with_tool_use_keeps_tool_use_stop_reason() {
        // thinking + tool_use case, stop_reason should be tool_use
        let mut ctx = StreamContext::new_with_thinking("test-model", 1, true);
        let _initial_events = ctx.generate_initial_events();

        let mut all_events = Vec::new();
        all_events.extend(ctx.process_assistant_response("<thinking>\nabc</thinking>"));
        all_events.extend(ctx.process_tool_use(&crate::kiro::model::events::ToolUseEvent {
            name: "test_tool".to_string(),
            tool_use_id: "tool_1".to_string(),
            input: "{}".to_string(),
            stop: true,
        }));
        all_events.extend(ctx.generate_final_events());

        let message_delta = all_events
            .iter()
            .find(|e| e.event == "message_delta")
            .expect("should have message_delta event");

        assert_eq!(
            message_delta.data["delta"]["stop_reason"], "tool_use",
            "stop_reason should be tool_use when tool_use is present"
        );
    }
}
