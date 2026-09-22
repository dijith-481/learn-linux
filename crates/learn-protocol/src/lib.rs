#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, missing_docs)]

//! Small versioned protobuf envelope and length-delimited framing.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use prost::Message;
use thiserror::Error;

/// Current protocol version.
pub const PROTOCOL_VERSION: u32 = 1;
/// Maximum accepted frame payload.
pub const MAX_FRAME_SIZE: usize = 8 * 1024 * 1024;

/// Wire envelope.
#[derive(Clone, PartialEq, Message)]
pub struct Envelope {
    /// Protocol version.
    #[prost(uint32, tag = "1")]
    pub protocol_version: u32,
    /// Client request/response correlation ID.
    #[prost(uint64, tag = "2")]
    pub message_id: u64,
    /// Envelope body.
    #[prost(oneof = "envelope::Body", tags = "10, 11, 12, 13")]
    pub body: Option<envelope::Body>,
}

/// Envelope variants.
pub mod envelope {
    /// Body variants.
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Body {
        /// Request.
        #[prost(message, tag = "10")]
        Request(super::Request),
        /// Response.
        #[prost(message, tag = "11")]
        Response(super::Response),
        /// Event.
        #[prost(message, tag = "12")]
        Event(super::Event),
        /// Cancellation.
        #[prost(message, tag = "13")]
        Cancel(super::Cancel),
    }
}

/// Request envelope body.
#[derive(Clone, PartialEq, Message)]
pub struct Request {
    /// Request kind.
    #[prost(oneof = "request::Kind", tags = "1, 2, 3, 4, 5, 6, 7, 8, 9")]
    pub kind: Option<request::Kind>,
}

/// Request variants.
pub mod request {
    /// Request kind.
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Kind {
        /// Handshake.
        #[prost(message, tag = "1")]
        Hello(super::HelloRequest),
        /// Get content status.
        #[prost(message, tag = "2")]
        Status(super::StatusRequest),
        /// Open a challenge session.
        #[prost(message, tag = "3")]
        OpenSession(super::OpenSessionRequest),
        /// Send terminal bytes.
        #[prost(message, tag = "4")]
        TerminalInput(super::TerminalInput),
        /// Reset a challenge.
        #[prost(message, tag = "5")]
        Reset(super::ResetRequest),
        /// Search local docs.
        #[prost(message, tag = "6")]
        SearchDocs(super::SearchDocsRequest),
        /// Request graph data.
        #[prost(message, tag = "7")]
        Graph(super::GraphRequest),
        /// Resize a terminal session.
        #[prost(message, tag = "8")]
        TerminalResize(super::TerminalResize),
        /// Close an active terminal session.
        #[prost(message, tag = "9")]
        CloseSession(super::CloseSession),
    }
}

/// Response envelope body.
#[derive(Clone, PartialEq, Message)]
pub struct Response {
    /// Whether the request succeeded.
    #[prost(bool, tag = "1")]
    pub ok: bool,
    /// Human-readable error when not successful.
    #[prost(string, tag = "2")]
    pub error: String,
    /// Optional response payload.
    #[prost(oneof = "response::Kind", tags = "10, 11, 12, 13, 14")]
    pub kind: Option<response::Kind>,
}

/// Response variants.
pub mod response {
    /// Response kind.
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Kind {
        /// Handshake response.
        #[prost(message, tag = "10")]
        Hello(super::HelloResponse),
        /// Runtime/content status.
        #[prost(message, tag = "11")]
        Status(super::StatusResponse),
        /// Challenge metadata.
        #[prost(message, tag = "12")]
        Challenge(super::ChallengeResponse),
        /// Search result payload.
        #[prost(message, tag = "13")]
        Docs(super::DocsResponse),
        /// Graph payload.
        #[prost(message, tag = "14")]
        Graph(super::GraphResponse),
    }
}

/// Event envelope body.
#[derive(Clone, PartialEq, Message)]
pub struct Event {
    /// Event kind.
    #[prost(oneof = "event::Kind", tags = "1, 2")]
    pub kind: Option<event::Kind>,
}

/// Event variants.
pub mod event {
    /// Event kind.
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Kind {
        /// Raw terminal output.
        #[prost(message, tag = "1")]
        TerminalOutput(super::TerminalOutput),
        /// Challenge state update.
        #[prost(message, tag = "2")]
        ChallengeState(super::ChallengeStateEvent),
    }
}

/// Cancel a request.
#[derive(Clone, PartialEq, Message)]
pub struct Cancel {
    /// ID being cancelled.
    #[prost(uint64, tag = "1")]
    pub message_id: u64,
}

/// Client handshake.
#[derive(Clone, PartialEq, Message)]
pub struct HelloRequest {
    /// Client version.
    #[prost(string, tag = "1")]
    pub client_version: String,
    /// Client kind.
    #[prost(string, tag = "2")]
    pub client_kind: String,
}

/// Server handshake.
#[derive(Clone, PartialEq, Message)]
pub struct HelloResponse {
    /// Server version.
    #[prost(string, tag = "1")]
    pub server_version: String,
    /// Minimum supported protocol version.
    #[prost(uint32, tag = "2")]
    pub protocol_version: u32,
    /// Runtime availability.
    #[prost(bool, tag = "3")]
    pub runtime_available: bool,
    /// Content pack version/hash.
    #[prost(string, tag = "4")]
    pub content_version: String,
}

/// Empty status request.
#[derive(Clone, PartialEq, Message)]
pub struct StatusRequest {}

/// Runtime/content status.
#[derive(Clone, PartialEq, Message)]
pub struct StatusResponse {
    /// Number of compiled challenges.
    #[prost(uint32, tag = "1")]
    pub challenge_count: u32,
    /// Number of completed challenges.
    #[prost(uint32, tag = "2")]
    pub completed_count: u32,
    /// Runtime diagnostic summary.
    #[prost(string, tag = "3")]
    pub runtime_summary: String,
}

/// Open a challenge.
#[derive(Clone, PartialEq, Message)]
pub struct OpenSessionRequest {
    /// Challenge ID.
    #[prost(string, tag = "1")]
    pub challenge_id: String,
    /// Terminal columns.
    #[prost(uint32, tag = "2")]
    pub columns: u32,
    /// Terminal rows.
    #[prost(uint32, tag = "3")]
    pub rows: u32,
}

/// Terminal input bytes.
#[derive(Clone, PartialEq, Message)]
pub struct TerminalInput {
    /// Session ID.
    #[prost(string, tag = "1")]
    pub session_id: String,
    /// Raw PTY bytes.
    #[prost(bytes = "vec", tag = "2")]
    pub data: Vec<u8>,
}

/// Terminal resize dimensions.
#[derive(Clone, PartialEq, Message)]
pub struct TerminalResize {
    /// Session ID.
    #[prost(string, tag = "1")]
    pub session_id: String,
    /// Terminal columns.
    #[prost(uint32, tag = "2")]
    pub columns: u32,
    /// Terminal rows.
    #[prost(uint32, tag = "3")]
    pub rows: u32,
}

/// Close-session request.
#[derive(Clone, PartialEq, Message)]
pub struct CloseSession {
    /// Session ID.
    #[prost(string, tag = "1")]
    pub session_id: String,
}

/// Reset a challenge.
#[derive(Clone, PartialEq, Message)]
pub struct ResetRequest {
    /// Challenge ID.
    #[prost(string, tag = "1")]
    pub challenge_id: String,
}

/// Search docs.
#[derive(Clone, PartialEq, Message)]
pub struct SearchDocsRequest {
    /// Search query.
    #[prost(string, tag = "1")]
    pub query: String,
    /// Maximum results.
    #[prost(uint32, tag = "2")]
    pub limit: u32,
}

/// Request graph.
#[derive(Clone, PartialEq, Message)]
pub struct GraphRequest {}

/// Challenge response.
#[derive(Clone, PartialEq, Message)]
pub struct ChallengeResponse {
    /// Stable ID.
    #[prost(string, tag = "1")]
    pub id: String,
    /// Title.
    #[prost(string, tag = "2")]
    pub title: String,
    /// Objective.
    #[prost(string, tag = "3")]
    pub objective: String,
    /// Session ID used for terminal input and output events.
    #[prost(string, tag = "4")]
    pub session_id: String,
}

/// Raw terminal output.
#[derive(Clone, PartialEq, Message)]
pub struct TerminalOutput {
    /// Session ID.
    #[prost(string, tag = "1")]
    pub session_id: String,
    /// Raw PTY bytes.
    #[prost(bytes = "vec", tag = "2")]
    pub data: Vec<u8>,
}

/// Challenge state event.
#[derive(Clone, PartialEq, Message)]
pub struct ChallengeStateEvent {
    /// Challenge ID.
    #[prost(string, tag = "1")]
    pub challenge_id: String,
    /// State name.
    #[prost(string, tag = "2")]
    pub state: String,
}

/// Search result response.
#[derive(Clone, PartialEq, Message)]
pub struct DocsResponse {
    /// Results.
    #[prost(message, repeated, tag = "1")]
    pub results: Vec<DocResult>,
}

/// One doc result.
#[derive(Clone, PartialEq, Message)]
pub struct DocResult {
    /// Document ID.
    #[prost(string, tag = "1")]
    pub id: String,
    /// Title.
    #[prost(string, tag = "2")]
    pub title: String,
    /// Snippet.
    #[prost(string, tag = "3")]
    pub snippet: String,
}

/// Graph response.
#[derive(Clone, PartialEq, Message)]
pub struct GraphResponse {
    /// Skills.
    #[prost(message, repeated, tag = "1")]
    pub skills: Vec<SkillNode>,
}

/// One graph skill.
#[derive(Clone, PartialEq, Message)]
pub struct SkillNode {
    /// ID.
    #[prost(string, tag = "1")]
    pub id: String,
    /// Title.
    #[prost(string, tag = "2")]
    pub title: String,
    /// Prerequisite IDs.
    #[prost(string, repeated, tag = "3")]
    pub prerequisites: Vec<String>,
}

/// Encode an envelope with a four-byte big-endian length prefix.
pub fn encode_frame(envelope: &Envelope) -> Result<Bytes, FrameError> {
    let mut payload = Vec::new();
    envelope.encode(&mut payload).map_err(FrameError::Encode)?;
    if payload.len() > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge(payload.len()));
    }
    let length = u32::try_from(payload.len()).map_err(|_| FrameError::TooLarge(payload.len()))?;
    let mut frame = BytesMut::with_capacity(payload.len() + 4);
    frame.put_u32(length);
    frame.extend_from_slice(&payload);
    Ok(frame.freeze())
}

/// Decode at most one complete frame from a mutable input buffer.
pub fn decode_frame(buffer: &mut BytesMut) -> Result<Option<Envelope>, FrameError> {
    if buffer.len() < 4 {
        return Ok(None);
    }
    let mut header = &buffer[..4];
    let length = header.get_u32() as usize;
    if length > MAX_FRAME_SIZE {
        return Err(FrameError::TooLarge(length));
    }
    if buffer.len() < 4 + length {
        return Ok(None);
    }
    buffer.advance(4);
    let payload = buffer.split_to(length);
    Envelope::decode(payload)
        .map(Some)
        .map_err(FrameError::Decode)
}

/// Framing/serialization error.
#[derive(Debug, Error)]
pub enum FrameError {
    /// Protobuf encoding failed.
    #[error("protobuf encode failed: {0}")]
    Encode(#[source] prost::EncodeError),
    /// Protobuf decoding failed.
    #[error("protobuf decode failed: {0}")]
    Decode(#[source] prost::DecodeError),
    /// A frame exceeds the protocol limit.
    #[error("frame has {0} bytes; maximum is {MAX_FRAME_SIZE}")]
    TooLarge(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip_preserves_bytes() {
        let envelope = Envelope {
            protocol_version: PROTOCOL_VERSION,
            message_id: 42,
            body: Some(envelope::Body::Event(Event {
                kind: Some(event::Kind::TerminalOutput(TerminalOutput {
                    session_id: "test".to_owned(),
                    data: vec![0, 255, 10],
                })),
            })),
        };
        let mut bytes = BytesMut::from(encode_frame(&envelope).expect("encode").as_ref());
        let decoded = decode_frame(&mut bytes).expect("decode").expect("complete");
        assert_eq!(decoded, envelope);
        assert!(bytes.is_empty());
    }
}
