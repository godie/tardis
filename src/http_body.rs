//! Bounded response-body reads shared by the local model-server clients.
//!
//! Whisper's JSON reply and Piper's WAV reply are parsed by different
//! code, but both must stop reading at a documented limit so a broken or
//! compromised endpoint cannot make the app buffer an unbounded body.
//! The LLM client keeps its own cap in
//! [`crate::processor::llm::MAX_RESPONSE_BYTES`]; this helper is what the
//! two local-audio clients share.

use std::io::Read;

use anyhow::{Result, anyhow};

/// Read at most `max_bytes` from `reader`.
///
/// Reads one byte past the limit so an oversized body is refused instead
/// of silently truncated: a cut WAV or JSON would otherwise fail later
/// with a misleading parse error. The error names the limit.
pub fn read_capped_body<R: Read>(reader: R, max_bytes: usize) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    // `take` leaves at most limit + 1 bytes to read, so the buffer cannot
    // grow past the cap even when the peer keeps sending.
    let mut limited = reader.take(max_bytes as u64 + 1);
    limited.read_to_end(&mut body)?;
    if body.len() > max_bytes {
        return Err(anyhow!(
            "response body exceeded {max_bytes} bytes; refusing to buffer it"
        ));
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, repeat};

    #[test]
    fn a_body_below_the_limit_is_returned_whole() {
        let body = read_capped_body(Cursor::new(b"hello".to_vec()), 16).unwrap();
        assert_eq!(body, b"hello");
    }

    #[test]
    fn a_body_exactly_at_the_limit_is_accepted() {
        let body = read_capped_body(Cursor::new(vec![7u8; 16]), 16).unwrap();
        assert_eq!(body, vec![7u8; 16]);
    }

    #[test]
    fn a_body_one_byte_over_the_limit_is_refused() {
        let err = read_capped_body(Cursor::new(vec![0u8; 17]), 16).unwrap_err();
        assert!(
            err.to_string().contains("exceeded 16 bytes"),
            "error must name the limit, got: {err}"
        );
    }

    /// An endless reply must fail at the limit instead of filling memory:
    /// `take` is what bounds this, so the test would hang or grow without
    /// it.
    #[test]
    fn an_endless_body_stops_at_the_limit() {
        let err = read_capped_body(repeat(0u8), 16).unwrap_err();
        assert!(
            err.to_string().contains("exceeded 16 bytes"),
            "error must name the limit, got: {err}"
        );
    }
}
