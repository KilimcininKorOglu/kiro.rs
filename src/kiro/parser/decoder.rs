//! AWS Event Stream streaming decoder
//!
//! Uses state machine to process streaming data, supports resumption and fault tolerance
//!
//! ## State Machine Design
//!
//! Based on kiro-kt project's state machine design, using a four-state model:
//!
//! ```text
//! ┌─────────────────┐
//! │      Ready      │  (Initial state, ready to receive data)
//! └────────┬────────┘
//!          │ feed() provides data
//!          ↓
//! ┌─────────────────┐
//! │     Parsing     │  decode() attempts to parse
//! └────────┬────────┘
//!          │
//!     ┌────┴────────────┐
//!     ↓                 ↓
//!  [Success]         [Failure]
//!     │                 │
//!     ↓                 ├─> error_count++
//! ┌─────────┐           │
//! │  Ready  │           ├─> error_count < max_errors?
//! └─────────┘           │    YES → Recovering → Ready
//!                       │    NO  ↓
//!                  ┌────────────┐
//!                  │   Stopped  │ (Terminal state)
//!                  └────────────┘
//! ```

use super::error::{ParseError, ParseResult};
use super::frame::{Frame, PRELUDE_SIZE, parse_frame};
use bytes::{Buf, BytesMut};

/// Default maximum buffer size (16 MB)
pub const DEFAULT_MAX_BUFFER_SIZE: usize = 16 * 1024 * 1024;

/// Default maximum consecutive errors
pub const DEFAULT_MAX_ERRORS: usize = 5;

/// Default initial buffer capacity
pub const DEFAULT_BUFFER_CAPACITY: usize = 8192;

/// Decoder state
///
/// Four-state model based on kiro-kt design:
/// - Ready: Ready state, can receive data
/// - Parsing: Currently parsing frame
/// - Recovering: Recovering (attempting to skip corrupted data)
/// - Stopped: Stopped (too many errors, terminal state)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderState {
    /// Ready, can receive data
    Ready,
    /// Currently parsing frame
    Parsing,
    /// Recovering (skipping corrupted data)
    Recovering,
    /// Stopped (too many errors)
    Stopped,
}

/// Streaming event decoder
///
/// Used to parse AWS Event Stream message frames from byte stream
///
/// # Example
///
/// ```rust,ignore
/// use kiro_rs::kiro::parser::EventStreamDecoder;
///
/// let mut decoder = EventStreamDecoder::new();
///
/// // Provide stream data
/// decoder.feed(chunk)?;
///
/// // Decode all available frames
/// for result in decoder.decode_iter() {
///     match result {
///         Ok(frame) => println!("Got frame: {:?}", frame.event_type()),
///         Err(e) => eprintln!("Parse error: {}", e),
///     }
/// }
/// ```
pub struct EventStreamDecoder {
    /// Internal buffer
    buffer: BytesMut,
    /// Current state
    state: DecoderState,
    /// Number of frames decoded
    frames_decoded: usize,
    /// Consecutive error count
    error_count: usize,
    /// Maximum consecutive errors
    max_errors: usize,
    /// Maximum buffer size
    max_buffer_size: usize,
    /// Bytes skipped (for debugging)
    bytes_skipped: usize,
}

impl Default for EventStreamDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl EventStreamDecoder {
    /// Create new decoder
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_BUFFER_CAPACITY)
    }

    /// Create decoder with specified buffer size
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: BytesMut::with_capacity(capacity),
            state: DecoderState::Ready,
            frames_decoded: 0,
            error_count: 0,
            max_errors: DEFAULT_MAX_ERRORS,
            max_buffer_size: DEFAULT_MAX_BUFFER_SIZE,
            bytes_skipped: 0,
        }
    }

    /// Create decoder with custom configuration
    pub fn with_config(capacity: usize, max_errors: usize, max_buffer_size: usize) -> Self {
        Self {
            buffer: BytesMut::with_capacity(capacity),
            state: DecoderState::Ready,
            frames_decoded: 0,
            error_count: 0,
            max_errors,
            max_buffer_size,
            bytes_skipped: 0,
        }
    }

    /// Feed data to decoder
    ///
    /// # Returns
    /// - `Ok(())` - Data added to buffer
    /// - `Err(BufferOverflow)` - Buffer is full
    pub fn feed(&mut self, data: &[u8]) -> ParseResult<()> {
        // Check buffer size limit
        let new_size = self.buffer.len() + data.len();
        if new_size > self.max_buffer_size {
            return Err(ParseError::BufferOverflow {
                size: new_size,
                max: self.max_buffer_size,
            });
        }

        self.buffer.extend_from_slice(data);

        // Recover from Recovering state to Ready
        if self.state == DecoderState::Recovering {
            self.state = DecoderState::Ready;
        }

        Ok(())
    }

    /// Try to decode next frame
    ///
    /// # Returns
    /// - `Ok(Some(frame))` - Successfully decoded a frame
    /// - `Ok(None)` - Insufficient data, need more data
    /// - `Err(e)` - Decode error
    pub fn decode(&mut self) -> ParseResult<Option<Frame>> {
        // If stopped, return error directly
        if self.state == DecoderState::Stopped {
            return Err(ParseError::TooManyErrors {
                count: self.error_count,
                last_error: "Decoder stopped".to_string(),
            });
        }

        // Buffer is empty, stay in Ready state
        if self.buffer.is_empty() {
            self.state = DecoderState::Ready;
            return Ok(None);
        }

        // Transition to Parsing state
        self.state = DecoderState::Parsing;

        match parse_frame(&self.buffer) {
            Ok(Some((frame, consumed))) => {
                // Successfully parsed
                self.buffer.advance(consumed);
                self.state = DecoderState::Ready;
                self.frames_decoded += 1;
                self.error_count = 0; // Reset consecutive error count
                Ok(Some(frame))
            }
            Ok(None) => {
                // Insufficient data, return to Ready state waiting for more data
                self.state = DecoderState::Ready;
                Ok(None)
            }
            Err(e) => {
                self.error_count += 1;
                let error_msg = e.to_string();

                // Check if exceeded maximum errors
                if self.error_count >= self.max_errors {
                    self.state = DecoderState::Stopped;
                    tracing::error!(
                        "Decoder stopped: {} consecutive errors, last error: {}",
                        self.error_count,
                        error_msg
                    );
                    return Err(ParseError::TooManyErrors {
                        count: self.error_count,
                        last_error: error_msg,
                    });
                }

                // Apply different recovery strategies based on error type
                self.try_recover(&e);
                self.state = DecoderState::Recovering;
                Err(e)
            }
        }
    }

    /// Create decode iterator
    pub fn decode_iter(&mut self) -> DecodeIter<'_> {
        DecodeIter { decoder: self }
    }

    /// Attempt fault-tolerant recovery
    ///
    /// Apply different recovery strategies based on error type (based on kiro-kt design):
    /// - Prelude phase errors (CRC failure, length anomaly): Skip 1 byte, try to find next frame boundary
    /// - Data phase errors (Message CRC failure, Header parse failure): Skip entire corrupted frame
    fn try_recover(&mut self, error: &ParseError) {
        if self.buffer.is_empty() {
            return;
        }

        match error {
            // Prelude phase errors: Frame boundary may be misaligned, scan byte by byte to find next valid boundary
            ParseError::PreludeCrcMismatch { .. }
            | ParseError::MessageTooSmall { .. }
            | ParseError::MessageTooLarge { .. } => {
                let skipped_byte = self.buffer[0];
                self.buffer.advance(1);
                self.bytes_skipped += 1;
                tracing::warn!(
                    "Prelude error recovery: skipped byte 0x{:02x} (total skipped {} bytes)",
                    skipped_byte,
                    self.bytes_skipped
                );
            }

            // Data phase errors: Frame boundary correct but data corrupted, skip entire frame
            ParseError::MessageCrcMismatch { .. } | ParseError::HeaderParseFailed(_) => {
                // Try to read total_length to skip entire frame
                if self.buffer.len() >= PRELUDE_SIZE {
                    let total_length = u32::from_be_bytes([
                        self.buffer[0],
                        self.buffer[1],
                        self.buffer[2],
                        self.buffer[3],
                    ]) as usize;

                    // Ensure total_length is reasonable and buffer has enough data
                    if total_length >= 16 && total_length <= self.buffer.len() {
                        tracing::warn!("Data error recovery: skipped corrupted frame ({} bytes)", total_length);
                        self.buffer.advance(total_length);
                        self.bytes_skipped += total_length;
                        return;
                    }
                }

                // Cannot determine frame length, fall back to byte-by-byte skip
                let skipped_byte = self.buffer[0];
                self.buffer.advance(1);
                self.bytes_skipped += 1;
                tracing::warn!(
                    "Data error recovery (fallback): skipped byte 0x{:02x} (total skipped {} bytes)",
                    skipped_byte,
                    self.bytes_skipped
                );
            }

            // Other errors: Skip byte by byte
            _ => {
                let skipped_byte = self.buffer[0];
                self.buffer.advance(1);
                self.bytes_skipped += 1;
                tracing::warn!(
                    "Generic error recovery: skipped byte 0x{:02x} (total skipped {} bytes)",
                    skipped_byte,
                    self.bytes_skipped
                );
            }
        }
    }

    // ==================== Lifecycle management methods ====================

    /// Reset decoder to initial state
    ///
    /// Clear buffer and all counters, restore to Ready state
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.state = DecoderState::Ready;
        self.frames_decoded = 0;
        self.error_count = 0;
        self.bytes_skipped = 0;
    }

    /// Get current state
    pub fn state(&self) -> DecoderState {
        self.state
    }

    /// Check if in Ready state
    pub fn is_ready(&self) -> bool {
        self.state == DecoderState::Ready
    }

    /// Check if in Stopped state
    pub fn is_stopped(&self) -> bool {
        self.state == DecoderState::Stopped
    }

    /// Check if in Recovering state
    pub fn is_recovering(&self) -> bool {
        self.state == DecoderState::Recovering
    }

    /// Get number of decoded frames
    pub fn frames_decoded(&self) -> usize {
        self.frames_decoded
    }

    /// Get current consecutive error count
    pub fn error_count(&self) -> usize {
        self.error_count
    }

    /// Get number of skipped bytes
    pub fn bytes_skipped(&self) -> usize {
        self.bytes_skipped
    }

    /// Get number of pending bytes in buffer
    pub fn buffer_len(&self) -> usize {
        self.buffer.len()
    }

    /// Try to resume from Stopped state
    ///
    /// Reset error count and transition to Ready state
    /// Note: Buffer contents are preserved, may still contain corrupted data
    pub fn try_resume(&mut self) {
        if self.state == DecoderState::Stopped {
            self.error_count = 0;
            self.state = DecoderState::Ready;
            tracing::info!("Decoder resumed from Stopped state");
        }
    }
}

/// Decode iterator
pub struct DecodeIter<'a> {
    decoder: &'a mut EventStreamDecoder,
}

impl<'a> Iterator for DecodeIter<'a> {
    type Item = ParseResult<Frame>;

    fn next(&mut self) -> Option<Self::Item> {
        // If in Stopped or Recovering state, stop iteration
        match self.decoder.state {
            DecoderState::Stopped => return None,
            DecoderState::Recovering => return None,
            _ => {}
        }

        match self.decoder.decode() {
            Ok(Some(frame)) => Some(Ok(frame)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoder_new() {
        let decoder = EventStreamDecoder::new();
        assert_eq!(decoder.state(), DecoderState::Ready);
        assert_eq!(decoder.frames_decoded(), 0);
        assert_eq!(decoder.error_count(), 0);
    }

    #[test]
    fn test_decoder_feed() {
        let mut decoder = EventStreamDecoder::new();
        assert!(decoder.feed(&[1, 2, 3, 4]).is_ok());
        assert_eq!(decoder.buffer_len(), 4);
    }

    #[test]
    fn test_decoder_buffer_overflow() {
        let mut decoder = EventStreamDecoder::with_config(1024, 5, 100);
        let result = decoder.feed(&[0u8; 101]);
        assert!(matches!(result, Err(ParseError::BufferOverflow { .. })));
    }

    #[test]
    fn test_decoder_insufficient_data() {
        let mut decoder = EventStreamDecoder::new();
        decoder.feed(&[0u8; 10]).unwrap();

        let result = decoder.decode();
        assert!(matches!(result, Ok(None)));
        assert_eq!(decoder.state(), DecoderState::Ready);
    }

    #[test]
    fn test_decoder_reset() {
        let mut decoder = EventStreamDecoder::new();
        decoder.feed(&[1, 2, 3, 4]).unwrap();

        decoder.reset();
        assert_eq!(decoder.state(), DecoderState::Ready);
        assert_eq!(decoder.buffer_len(), 0);
        assert_eq!(decoder.frames_decoded(), 0);
    }

    #[test]
    fn test_decoder_state_transitions() {
        let decoder = EventStreamDecoder::new();

        // Initial state
        assert!(decoder.is_ready());
        assert!(!decoder.is_stopped());
        assert!(!decoder.is_recovering());
    }

    #[test]
    fn test_decoder_try_resume() {
        let mut decoder = EventStreamDecoder::new();

        // Manually set to Stopped state for testing
        decoder.state = DecoderState::Stopped;
        decoder.error_count = 5;

        decoder.try_resume();
        assert!(decoder.is_ready());
        assert_eq!(decoder.error_count(), 0);
    }
}
