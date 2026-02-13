//! Anthropic API Handler functions

use std::convert::Infallible;

use crate::kiro::model::events::Event;
use crate::kiro::model::requests::kiro::KiroRequest;
use crate::kiro::parser::decoder::EventStreamDecoder;
use crate::token;
use axum::{
    Json as JsonExtractor,
    body::Body,
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Json, Response},
};
use bytes::Bytes;
use futures::{Stream, StreamExt, stream};
use serde_json::json;
use std::time::Duration;
use tokio::time::interval;
use uuid::Uuid;

use super::converter::{ConversionError, convert_request, inject_agentic_prompt};
use super::middleware::AppState;
use super::stream::{BufferedStreamContext, SseEvent, StreamContext};
use super::types::{CountTokensRequest, CountTokensResponse, ErrorResponse, MessagesRequest, Model, ModelsResponse, OutputConfig, Thinking};
use super::websearch;

/// Convert Kiro API error to Anthropic-compatible error response
/// 
/// Maps Kiro error messages to appropriate Anthropic error types and status codes
/// to ensure client compatibility (e.g., Claude Code auto-compress triggers)
fn convert_kiro_error_to_response(error_message: &str) -> Response {
    let error_lower = error_message.to_lowercase();
    
    // Check for quota exhausted errors (all credentials used up)
    if error_lower.contains("all credentials exhausted") || error_lower.contains("credentials quota") {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse::new(
                "rate_limit_error",
                "All credentials quota exhausted. Please wait for quota reset or add new credentials.",
            )),
        )
            .into_response();
    }
    
    // Check for context/content length errors - these should trigger client compress
    if error_lower.contains("improperly formed")
        || error_lower.contains("content length")
        || error_lower.contains("too long")
        || error_lower.contains("context")
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_request_error",
                error_message,
            )),
        )
            .into_response();
    }
    
    // Check for rate limit errors
    if error_lower.contains("rate limit") || error_lower.contains("throttl") {
        return (
            StatusCode::TOO_MANY_REQUESTS,
            Json(ErrorResponse::new(
                "rate_limit_error",
                error_message,
            )),
        )
            .into_response();
    }
    
    // Check for overloaded errors
    if error_lower.contains("overload") || error_lower.contains("capacity") {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse::new(
                "overloaded_error",
                error_message,
            )),
        )
            .into_response();
    }
    
    // Default: return as api_error with BAD_GATEWAY
    (
        StatusCode::BAD_GATEWAY,
        Json(ErrorResponse::new(
            "api_error",
            format!("Upstream API call failed: {}", error_message),
        )),
    )
        .into_response()
}

/// GET /v1/models
///
/// Returns the list of available models
pub async fn get_models() -> impl IntoResponse {
    tracing::info!("Received GET /v1/models request");

    let models = vec![
        Model {
            id: "claude-sonnet-4-5-20250929".to_string(),
            object: "model".to_string(),
            created: 1727568000,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Sonnet 4.5".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-sonnet-4-5-20250929-thinking".to_string(),
            object: "model".to_string(),
            created: 1727568000,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Sonnet 4.5 (Thinking)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-opus-4-5-20251101".to_string(),
            object: "model".to_string(),
            created: 1730419200,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.5".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-opus-4-5-20251101-thinking".to_string(),
            object: "model".to_string(),
            created: 1730419200,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.5 (Thinking)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            thinking: Some(true),
        },
        // Opus 4.6 - 200K context (standard)
        Model {
            id: "claude-opus-4-6".to_string(),
            object: "model".to_string(),
            created: 1770314400,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.6".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(128_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-opus-4-6-thinking".to_string(),
            object: "model".to_string(),
            created: 1770314400,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.6 (Thinking)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(128_000),
            thinking: Some(true),
        },
        // Opus 4.6 - 1M context (large projects)
        Model {
            id: "claude-opus-4-6-1m".to_string(),
            object: "model".to_string(),
            created: 1770314400,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.6 (1M Context)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(1_000_000),
            max_completion_tokens: Some(128_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-opus-4-6-1m-thinking".to_string(),
            object: "model".to_string(),
            created: 1770314400,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.6 (1M Context, Thinking)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(1_000_000),
            max_completion_tokens: Some(128_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-haiku-4-5-20251001".to_string(),
            object: "model".to_string(),
            created: 1727740800,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Haiku 4.5".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-haiku-4-5-20251001-thinking".to_string(),
            object: "model".to_string(),
            created: 1727740800,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Haiku 4.5 (Thinking)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            thinking: Some(true),
        },
        // Agentic variants - with chunked write system prompt
        Model {
            id: "claude-sonnet-4-5-20250929-agentic".to_string(),
            object: "model".to_string(),
            created: 1727568000,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Sonnet 4.5 (Agentic)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-opus-4-5-20251101-agentic".to_string(),
            object: "model".to_string(),
            created: 1730419200,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.5 (Agentic)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-opus-4-6-agentic".to_string(),
            object: "model".to_string(),
            created: 1770314400,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.6 (Agentic)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(128_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-opus-4-6-1m-agentic".to_string(),
            object: "model".to_string(),
            created: 1770314400,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Opus 4.6 (1M, Agentic)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(1_000_000),
            max_completion_tokens: Some(128_000),
            thinking: Some(true),
        },
        Model {
            id: "claude-haiku-4-5-20251001-agentic".to_string(),
            object: "model".to_string(),
            created: 1727740800,
            owned_by: "anthropic".to_string(),
            display_name: "Claude Haiku 4.5 (Agentic)".to_string(),
            model_type: "chat".to_string(),
            max_tokens: 32000,
            context_length: Some(200_000),
            max_completion_tokens: Some(64_000),
            thinking: Some(true),
        },
    ];

    Json(ModelsResponse {
        object: "list".to_string(),
        data: models,
    })
}

/// POST /v1/messages
///
/// Create a message (conversation)
pub async fn post_messages(
    State(state): State<AppState>,
    JsonExtractor(mut payload): JsonExtractor<MessagesRequest>,
) -> Response {
    tracing::info!(
        model = %payload.model,
        max_tokens = %payload.max_tokens,
        stream = %payload.stream,
        message_count = %payload.messages.len(),
        "Received POST /v1/messages request"
    );
    // Check if KiroProvider is available
    let provider = match &state.kiro_provider {
        Some(p) => p.clone(),
        None => {
            tracing::error!("KiroProvider not configured");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::new(
                    "service_unavailable",
                    "Kiro API provider not configured",
                )),
            )
                .into_response();
        }
    };

    // Detect if model name contains thinking suffix, if so override thinking config
    let thinking_suffix = state.config.thinking_suffix();
    override_thinking_from_model_name(&mut payload, thinking_suffix);

    // Detect if model name contains agentic suffix, if so inject agentic prompt
    let is_agentic = detect_and_strip_agentic_suffix(&mut payload);
    if is_agentic {
        let current_system = payload
            .system
            .as_ref()
            .map(|msgs| msgs.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        let new_system = inject_agentic_prompt(&current_system);
        payload.system = Some(vec![super::types::SystemMessage { text: new_system }]);
    }

    // Check if this is a WebSearch request
    if websearch::has_web_search_tool(&payload) {
        tracing::info!("WebSearch tool detected, routing to WebSearch handler");

        // Estimate input tokens
        let input_tokens = token::count_all_tokens(
            payload.model.clone(),
            payload.system.clone(),
            payload.messages.clone(),
            payload.tools.clone(),
        ) as i32;

        return websearch::handle_websearch_request(provider, &payload, input_tokens).await;
    }

    // Convert request
    let conversion_result = match convert_request(&payload) {
        Ok(result) => result,
        Err(e) => {
            let (error_type, message) = match &e {
                ConversionError::UnsupportedModel(model) => {
                    ("invalid_request_error", format!("Model not supported: {}", model))
                }
                ConversionError::EmptyMessages => {
                    ("invalid_request_error", "Message list is empty".to_string())
                }
            };
            tracing::warn!("Request conversion failed: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(error_type, message)),
            )
                .into_response();
        }
    };

    // Log history message count for debugging
    let history_count = conversion_result.conversation_state.history.len();
    tracing::debug!(
        original_messages = %payload.messages.len(),
        converted_history = %history_count,
        "Request conversion completed"
    );

    // Build Kiro request
    let kiro_request = KiroRequest {
        conversation_state: conversion_result.conversation_state,
        profile_arn: state.profile_arn.clone(),
    };

    let request_body = match serde_json::to_string(&kiro_request) {
        Ok(body) => body,
        Err(e) => {
            tracing::error!("Failed to serialize request: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "internal_error",
                    format!("Failed to serialize request: {}", e),
                )),
            )
                .into_response();
        }
    };

    // Request body size pre-check
    let max_body = state.config.max_request_body_bytes;
    if max_body > 0 && request_body.len() > max_body {
        tracing::warn!(
            request_body_bytes = request_body.len(),
            threshold = max_body,
            "Request too large ({} bytes, limit {}). Reduce conversation history or tool output.",
            request_body.len(),
            max_body
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_request_error",
                "Input is too long for model context window.",
            )),
        )
            .into_response();
    }

    tracing::debug!("Kiro request body: {}", request_body);

    // Estimate input tokens
    let input_tokens = token::count_all_tokens(
        payload.model.clone(),
        payload.system,
        payload.messages,
        payload.tools,
    ) as i32;

    // Check if thinking is enabled
    let thinking_enabled = payload
        .thinking
        .as_ref()
        .map(|t| t.is_enabled())
        .unwrap_or(false);

    if payload.stream {
        // Streaming response
        handle_stream_request(
            provider,
            &request_body,
            &payload.model,
            input_tokens,
            thinking_enabled,
        )
        .await
    } else {
        // Non-streaming response
        handle_non_stream_request(provider, &request_body, &payload.model, input_tokens).await
    }
}

/// Handle streaming request
async fn handle_stream_request(
    provider: std::sync::Arc<crate::kiro::provider::KiroProvider>,
    request_body: &str,
    model: &str,
    input_tokens: i32,
    thinking_enabled: bool,
) -> Response {
    // Call Kiro API (supports multi-credential failover)
    let response = match provider.call_api_stream(request_body).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Kiro API call failed: {}", e);
            return convert_kiro_error_to_response(&e.to_string());
        }
    };

    // Create stream processing context
    let mut ctx = StreamContext::new_with_thinking(model, input_tokens, thinking_enabled);

    // Generate initial events
    let initial_events = ctx.generate_initial_events();

    // Create SSE stream
    let stream = create_sse_stream(response, ctx, initial_events);

    // Return SSE response
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// Ping event interval (25 seconds)
const PING_INTERVAL_SECS: u64 = 25;

/// Create ping event SSE string
fn create_ping_sse() -> Bytes {
    Bytes::from("event: ping\ndata: {\"type\": \"ping\"}\n\n")
}

/// Create SSE event stream
fn create_sse_stream(
    response: reqwest::Response,
    ctx: StreamContext,
    initial_events: Vec<SseEvent>,
) -> impl Stream<Item = Result<Bytes, Infallible>> {
    // Send initial events first
    let initial_stream = stream::iter(
        initial_events
            .into_iter()
            .map(|e| Ok(Bytes::from(e.to_sse_string()))),
    );

    // Then process Kiro response stream, sending ping keepalive every 25 seconds
    let body_stream = response.bytes_stream();

    let processing_stream = stream::unfold(
        (body_stream, ctx, EventStreamDecoder::new(), false, interval(Duration::from_secs(PING_INTERVAL_SECS))),
        |(mut body_stream, mut ctx, mut decoder, finished, mut ping_interval)| async move {
            if finished {
                return None;
            }

            // Use select! to wait for both data and ping timer
            tokio::select! {
                // Process data stream
                chunk_result = body_stream.next() => {
                    match chunk_result {
                        Some(Ok(chunk)) => {
                            // Decode events
                            if let Err(e) = decoder.feed(&chunk) {
                                tracing::warn!("Buffer overflow: {}", e);
                            }

                            let mut events = Vec::new();
                            for result in decoder.decode_iter() {
                                match result {
                                    Ok(frame) => {
                                        if let Ok(event) = Event::from_frame(frame) {
                                            let sse_events = ctx.process_kiro_event(&event);
                                            events.extend(sse_events);
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to decode event: {}", e);
                                    }
                                }
                            }

                            // Convert to SSE byte stream
                            let bytes: Vec<Result<Bytes, Infallible>> = events
                                .into_iter()
                                .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                .collect();

                            Some((stream::iter(bytes), (body_stream, ctx, decoder, false, ping_interval)))
                        }
                        Some(Err(e)) => {
                            tracing::error!("Failed to read response stream: {}", e);
                            // Send final events and end
                            let final_events = ctx.generate_final_events();
                            let bytes: Vec<Result<Bytes, Infallible>> = final_events
                                .into_iter()
                                .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                .collect();
                            Some((stream::iter(bytes), (body_stream, ctx, decoder, true, ping_interval)))
                        }
                        None => {
                            // Stream ended, send final events
                            let final_events = ctx.generate_final_events();
                            let bytes: Vec<Result<Bytes, Infallible>> = final_events
                                .into_iter()
                                .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                .collect();
                            Some((stream::iter(bytes), (body_stream, ctx, decoder, true, ping_interval)))
                        }
                    }
                }
                // Send ping keepalive
                _ = ping_interval.tick() => {
                    tracing::trace!("Sending ping keepalive event");
                    let bytes: Vec<Result<Bytes, Infallible>> = vec![Ok(create_ping_sse())];
                    Some((stream::iter(bytes), (body_stream, ctx, decoder, false, ping_interval)))
                }
            }
        },
    )
    .flatten();

    initial_stream.chain(processing_stream)
}

/// Context window size (200k tokens)
const CONTEXT_WINDOW_SIZE: i32 = 200_000;

/// Handle non-streaming request
async fn handle_non_stream_request(
    provider: std::sync::Arc<crate::kiro::provider::KiroProvider>,
    request_body: &str,
    model: &str,
    input_tokens: i32,
) -> Response {
    // Call Kiro API (supports multi-credential failover)
    let response = match provider.call_api(request_body).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Kiro API call failed: {}", e);
            return convert_kiro_error_to_response(&e.to_string());
        }
    };

    // Read response body
    let body_bytes = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!("Failed to read response body: {}", e);
            return (
                StatusCode::BAD_GATEWAY,
                Json(ErrorResponse::new(
                    "api_error",
                    format!("Failed to read response: {}", e),
                )),
            )
                .into_response();
        }
    };

    // Parse event stream
    let mut decoder = EventStreamDecoder::new();
    if let Err(e) = decoder.feed(&body_bytes) {
        tracing::warn!("Buffer overflow: {}", e);
    }

    let mut text_content = String::new();
    let mut tool_uses: Vec<serde_json::Value> = Vec::new();
    let mut has_tool_use = false;
    let mut stop_reason = "end_turn".to_string();
    // Actual input tokens calculated from contextUsageEvent
    let mut context_input_tokens: Option<i32> = None;

    // Collect incremental JSON for tool calls
    let mut tool_json_buffers: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    for result in decoder.decode_iter() {
        match result {
            Ok(frame) => {
                if let Ok(event) = Event::from_frame(frame) {
                    match event {
                        Event::AssistantResponse(resp) => {
                            text_content.push_str(&resp.content);
                        }
                        Event::ToolUse(tool_use) => {
                            has_tool_use = true;

                            // Accumulate tool's JSON input
                            let buffer = tool_json_buffers
                                .entry(tool_use.tool_use_id.clone())
                                .or_insert_with(String::new);
                            buffer.push_str(&tool_use.input);

                            // If this is a complete tool call, add to list
                            if tool_use.stop {
                                let input: serde_json::Value = serde_json::from_str(buffer)
                                    .unwrap_or_else(|e| {
                                        tracing::warn!(
                                            "Failed to parse tool input JSON: {}, tool_use_id: {}, raw content: {}",
                                            e, tool_use.tool_use_id, buffer
                                        );
                                        serde_json::json!({})
                                    });

                                tool_uses.push(json!({
                                    "type": "tool_use",
                                    "id": tool_use.tool_use_id,
                                    "name": tool_use.name,
                                    "input": input
                                }));
                            }
                        }
                        Event::ContextUsage(context_usage) => {
                            // Calculate actual input_tokens from context usage percentage
                            // Formula: percentage * 200000 / 100 = percentage * 2000
                            let actual_input_tokens = (context_usage.context_usage_percentage
                                * (CONTEXT_WINDOW_SIZE as f64)
                                / 100.0)
                                as i32;
                            context_input_tokens = Some(actual_input_tokens);
                            // When context usage reaches 100%, set stop_reason to model_context_window_exceeded
                            if context_usage.context_usage_percentage >= 100.0 {
                                stop_reason = "model_context_window_exceeded".to_string();
                            }
                            tracing::debug!(
                                "Received contextUsageEvent: {}%, calculated input_tokens: {}",
                                context_usage.context_usage_percentage,
                                actual_input_tokens
                            );
                        }
                        Event::Exception { exception_type, .. } => {
                            if exception_type == "ContentLengthExceededException" {
                                stop_reason = "max_tokens".to_string();
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Failed to decode event: {}", e);
            }
        }
    }

    // Determine stop_reason
    if has_tool_use && stop_reason == "end_turn" {
        stop_reason = "tool_use".to_string();
    }

    // Build response content
    let mut content: Vec<serde_json::Value> = Vec::new();

    if !text_content.is_empty() {
        content.push(json!({
            "type": "text",
            "text": text_content
        }));
    }

    content.extend(tool_uses);

    // Estimate output tokens
    let output_tokens = token::estimate_output_tokens(&content);

    // Use input_tokens calculated from contextUsageEvent, fallback to estimate if not available
    let final_input_tokens = context_input_tokens.unwrap_or(input_tokens);

    // Build Anthropic response
    let response_body = json!({
        "id": format!("msg_{}", Uuid::new_v4().to_string().replace('-', "")),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": model,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": {
            "input_tokens": final_input_tokens,
            "output_tokens": output_tokens
        }
    });

    (StatusCode::OK, Json(response_body)).into_response()
}

/// Detect if model name contains thinking suffix, if so override thinking config
///
/// - Opus 4.6: Override to adaptive type
/// - Other models: Override to enabled type
/// - budget_tokens fixed at 20000
/// - Removes the suffix from model name
fn override_thinking_from_model_name(payload: &mut MessagesRequest, thinking_suffix: &str) {
    let model_lower = payload.model.to_lowercase();
    let suffix_lower = thinking_suffix.to_lowercase();
    
    if !model_lower.ends_with(&suffix_lower) {
        return;
    }

    // Remove suffix from model name
    let actual_model = payload.model[..payload.model.len() - thinking_suffix.len()].to_string();
    let actual_model_lower = actual_model.to_lowercase();

    let is_opus_4_6 =
        actual_model_lower.contains("opus") && (actual_model_lower.contains("4-6") || actual_model_lower.contains("4.6"));

    let thinking_type = if is_opus_4_6 {
        "adaptive"
    } else {
        "enabled"
    };

    tracing::info!(
        original_model = %payload.model,
        actual_model = %actual_model,
        thinking_type = thinking_type,
        "Model name contains thinking suffix, overriding thinking config"
    );

    // Update model name (remove suffix)
    payload.model = actual_model;

    payload.thinking = Some(Thinking {
        thinking_type: thinking_type.to_string(),
        budget_tokens: 20000,
    });
    
    if is_opus_4_6 {
        payload.output_config = Some(OutputConfig {
            effort: "high".to_string(),
        });
    }
}

/// Detect if model name contains agentic suffix, if so strip it and return true
///
/// Returns true if agentic mode should be enabled
fn detect_and_strip_agentic_suffix(payload: &mut MessagesRequest) -> bool {
    let model_lower = payload.model.to_lowercase();
    
    if !model_lower.ends_with("-agentic") {
        return false;
    }

    // Remove suffix from model name
    let actual_model = payload.model[..payload.model.len() - 8].to_string();

    tracing::info!(
        original_model = %payload.model,
        actual_model = %actual_model,
        "Model name contains agentic suffix, enabling agentic mode"
    );

    payload.model = actual_model;
    true
}

/// POST /v1/messages/count_tokens
///
/// Calculate the token count for messages
pub async fn count_tokens(
    JsonExtractor(payload): JsonExtractor<CountTokensRequest>,
) -> impl IntoResponse {
    tracing::info!(
        model = %payload.model,
        message_count = %payload.messages.len(),
        "Received POST /v1/messages/count_tokens request"
    );

    let total_tokens = token::count_all_tokens(
        payload.model,
        payload.system,
        payload.messages,
        payload.tools,
    ) as i32;

    Json(CountTokensResponse {
        input_tokens: total_tokens.max(1) as i32,
    })
}

/// POST /cc/v1/messages
///
/// Claude Code compatible endpoint, differs from /v1/messages in that:
/// - Streaming response waits for kiro to return contextUsageEvent before sending message_start
/// - input_tokens in message_start is the accurate value calculated from contextUsageEvent
pub async fn post_messages_cc(
    State(state): State<AppState>,
    JsonExtractor(mut payload): JsonExtractor<MessagesRequest>,
) -> Response {
    tracing::info!(
        model = %payload.model,
        max_tokens = %payload.max_tokens,
        stream = %payload.stream,
        message_count = %payload.messages.len(),
        "Received POST /cc/v1/messages request"
    );

    // Check if KiroProvider is available
    let provider = match &state.kiro_provider {
        Some(p) => p.clone(),
        None => {
            tracing::error!("KiroProvider not configured");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(ErrorResponse::new(
                    "service_unavailable",
                    "Kiro API provider not configured",
                )),
            )
                .into_response();
        }
    };

    // Detect if model name contains thinking suffix, if so override thinking config
    let thinking_suffix = state.config.thinking_suffix();
    override_thinking_from_model_name(&mut payload, thinking_suffix);

    // Detect if model name contains agentic suffix, if so inject agentic prompt
    let is_agentic = detect_and_strip_agentic_suffix(&mut payload);
    if is_agentic {
        let current_system = payload
            .system
            .as_ref()
            .map(|msgs| msgs.iter().map(|m| m.text.as_str()).collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        let new_system = inject_agentic_prompt(&current_system);
        payload.system = Some(vec![super::types::SystemMessage { text: new_system }]);
    }

    // Check if this is a WebSearch request
    if websearch::has_web_search_tool(&payload) {
        tracing::info!("WebSearch tool detected, routing to WebSearch handler");

        // Estimate input tokens
        let input_tokens = token::count_all_tokens(
            payload.model.clone(),
            payload.system.clone(),
            payload.messages.clone(),
            payload.tools.clone(),
        ) as i32;

        return websearch::handle_websearch_request(provider, &payload, input_tokens).await;
    }

    // Convert request
    let conversion_result = match convert_request(&payload) {
        Ok(result) => result,
        Err(e) => {
            let (error_type, message) = match &e {
                ConversionError::UnsupportedModel(model) => {
                    ("invalid_request_error", format!("Model not supported: {}", model))
                }
                ConversionError::EmptyMessages => {
                    ("invalid_request_error", "Message list is empty".to_string())
                }
            };
            tracing::warn!("Request conversion failed: {}", e);
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse::new(error_type, message)),
            )
                .into_response();
        }
    };

    // Log history message count for debugging
    let history_count = conversion_result.conversation_state.history.len();
    tracing::debug!(
        original_messages = %payload.messages.len(),
        converted_history = %history_count,
        "Request conversion completed"
    );

    // Build Kiro request
    let kiro_request = KiroRequest {
        conversation_state: conversion_result.conversation_state,
        profile_arn: state.profile_arn.clone(),
    };

    let request_body = match serde_json::to_string(&kiro_request) {
        Ok(body) => body,
        Err(e) => {
            tracing::error!("Failed to serialize request: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse::new(
                    "internal_error",
                    format!("Failed to serialize request: {}", e),
                )),
            )
                .into_response();
        }
    };

    // Request body size pre-check
    let max_body = state.config.max_request_body_bytes;
    if max_body > 0 && request_body.len() > max_body {
        tracing::warn!(
            request_body_bytes = request_body.len(),
            threshold = max_body,
            "Request too large ({} bytes, limit {}). Reduce conversation history or tool output.",
            request_body.len(),
            max_body
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "invalid_request_error",
                "Input is too long for model context window.",
            )),
        )
            .into_response();
    }

    tracing::debug!("Kiro request body: {}", request_body);

    // Estimate input tokens
    let input_tokens = token::count_all_tokens(
        payload.model.clone(),
        payload.system,
        payload.messages,
        payload.tools,
    ) as i32;

    // Check if thinking is enabled
    let thinking_enabled = payload
        .thinking
        .as_ref()
        .map(|t| t.is_enabled())
        .unwrap_or(false);

    if payload.stream {
        // Streaming response (buffered mode)
        handle_stream_request_buffered(
            provider,
            &request_body,
            &payload.model,
            input_tokens,
            thinking_enabled,
        )
        .await
    } else {
        // Non-streaming response (reuse existing logic, already uses correct input_tokens)
        handle_non_stream_request(provider, &request_body, &payload.model, input_tokens).await
    }
}

/// Handle streaming request (buffered version)
///
/// Unlike `handle_stream_request`, this function buffers all events until stream ends,
/// then generates message_start event with correct input_tokens calculated from contextUsageEvent.
async fn handle_stream_request_buffered(
    provider: std::sync::Arc<crate::kiro::provider::KiroProvider>,
    request_body: &str,
    model: &str,
    estimated_input_tokens: i32,
    thinking_enabled: bool,
) -> Response {
    // Call Kiro API (supports multi-credential failover)
    let response = match provider.call_api_stream(request_body).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Kiro API call failed: {}", e);
            return convert_kiro_error_to_response(&e.to_string());
        }
    };

    // Create buffered stream processing context
    let ctx = BufferedStreamContext::new(model, estimated_input_tokens, thinking_enabled);

    // Create buffered SSE stream
    let stream = create_buffered_sse_stream(response, ctx);

    // Return SSE response
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header(header::CONNECTION, "keep-alive")
        .body(Body::from_stream(stream))
        .unwrap()
}

/// Create buffered SSE event stream
///
/// Workflow:
/// 1. Wait for upstream stream to complete, only sending ping keepalive signals during this time
/// 2. Process all Kiro events using StreamContext's event processing logic, cache results
/// 3. After stream ends, correct message_start event with correct input_tokens
/// 4. Send all events at once
fn create_buffered_sse_stream(
    response: reqwest::Response,
    ctx: BufferedStreamContext,
) -> impl Stream<Item = Result<Bytes, Infallible>> {
    let body_stream = response.bytes_stream();

    stream::unfold(
        (
            body_stream,
            ctx,
            EventStreamDecoder::new(),
            false,
            interval(Duration::from_secs(PING_INTERVAL_SECS)),
        ),
        |(mut body_stream, mut ctx, mut decoder, finished, mut ping_interval)| async move {
            if finished {
                return None;
            }

            loop {
                tokio::select! {
                    // Use biased mode, prioritize checking ping timer
                    // Avoid ping being "starved" when upstream chunks are dense
                    biased;

                    // Prioritize ping keepalive (only data sent during waiting period)
                    _ = ping_interval.tick() => {
                        tracing::trace!("Sending ping keepalive event (buffered mode)");
                        let bytes: Vec<Result<Bytes, Infallible>> = vec![Ok(create_ping_sse())];
                        return Some((stream::iter(bytes), (body_stream, ctx, decoder, false, ping_interval)));
                    }

                    // Then process data stream
                    chunk_result = body_stream.next() => {
                        match chunk_result {
                            Some(Ok(chunk)) => {
                                // Decode events
                                if let Err(e) = decoder.feed(&chunk) {
                                    tracing::warn!("Buffer overflow: {}", e);
                                }

                                for result in decoder.decode_iter() {
                                    match result {
                                        Ok(frame) => {
                                            if let Ok(event) = Event::from_frame(frame) {
                                                // Buffer events (reuse StreamContext's processing logic)
                                                ctx.process_and_buffer(&event);
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("Failed to decode event: {}", e);
                                        }
                                    }
                                }
                                // Continue reading next chunk, don't send any data
                            }
                            Some(Err(e)) => {
                                tracing::error!("Failed to read response stream: {}", e);
                                // Error occurred, finish processing and return all events
                                let all_events = ctx.finish_and_get_all_events();
                                let bytes: Vec<Result<Bytes, Infallible>> = all_events
                                    .into_iter()
                                    .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                    .collect();
                                return Some((stream::iter(bytes), (body_stream, ctx, decoder, true, ping_interval)));
                            }
                            None => {
                                // Stream ended, finish processing and return all events (with corrected input_tokens)
                                let all_events = ctx.finish_and_get_all_events();
                                let bytes: Vec<Result<Bytes, Infallible>> = all_events
                                    .into_iter()
                                    .map(|e| Ok(Bytes::from(e.to_sse_string())))
                                    .collect();
                                return Some((stream::iter(bytes), (body_stream, ctx, decoder, true, ping_interval)));
                            }
                        }
                    }
                }
            }
        },
    )
    .flatten()
}
