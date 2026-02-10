//! Anthropic API compatible service module
//!
//! Provides HTTP service endpoints compatible with the Anthropic Claude API.
//!
//! # Supported endpoints
//!
//! ## Standard endpoints (/v1)
//! - `GET /v1/models` - Get list of available models
//! - `POST /v1/messages` - Create message (conversation)
//! - `POST /v1/messages/count_tokens` - Calculate token count
//!
//! ## Claude Code compatible endpoints (/cc/v1)
//! - `POST /cc/v1/messages` - Create message (streaming response waits for contextUsageEvent before sending message_start, ensuring accurate input_tokens)
//! - `POST /cc/v1/messages/count_tokens` - Calculate token count (same as /v1)
//!
//! # Usage example
//! ```rust,ignore
//! use kiro_rs::anthropic;
//!
//! let app = anthropic::create_router("your-api-key");
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! axum::serve(listener, app).await?;
//! ```

mod converter;
mod handlers;
mod middleware;
mod router;
mod stream;
pub mod types;
mod websearch;

pub use router::create_router_with_provider;
