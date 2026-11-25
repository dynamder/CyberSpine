//! State and plumbing every plugin server shares.
//!
//! The messages differ — a body answers `Sample`, an arbiter answers `Tick` —
//! but a server on either side is one frame in and one frame out, and both have
//! the same shared concerns: what a *retry* means on one connection, what a
//! socket-serving loop needs from the thing it drives, and what a panic inside
//! user code costs.

use std::panic::{AssertUnwindSafe, catch_unwind};

/// One transport's session with a server.
///
/// The retry that needs deduplication is a *transport buffer* retry: the ABI
/// reports a too-small reply buffer after running the request, so the caller
/// re-sends the same frame. That is a fact about one connection — a TCP client
/// numbers its requests from 1 again every time it connects — so the cursor
/// lives here and not on the server.
///
/// A server whose `seq` is narrower (a body's is `u32`) widens it for the
/// comparison; the cursor only has to recognize the frame the previous reply was
/// built from, not to reproduce the protocol's numbering.
#[derive(Debug, Default)]
pub struct Connection {
    last_seq: Option<u64>,
}

impl Connection {
    /// Whether this request is the one whose reply is in the server's buffer.
    pub fn is_repeated(&self, seq: u64) -> bool {
        self.last_seq == Some(seq)
    }

    /// Record the frame whose reply is now in the server's buffer.
    ///
    /// Called after the reply exists, never before: a request that panicked has
    /// no reply, and a retry of it must run rather than hand back a stale frame.
    pub fn commit(&mut self, seq: u64) {
        self.last_seq = Some(seq);
    }
}

/// The text inside a panic payload.
///
/// A transport that catches a plugin panic still has to tell the other side what
/// happened; the payload is the only place that sentence lives.
pub fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "the plugin panicked".to_owned())
}

/// Trim a failure message so the frame that carries it always fits.
///
/// The text comes from the plugin, which has no length bound; a refusal still
/// has to be a frame the peer can read. The prefix survives and the cut is named.
pub fn cap_message(message: &mut String) {
    const SUFFIX: &str = "…(truncated)";
    // MessagePack adds a handful of bytes around the string; 64 is well above
    // the array/seq/code/string headers.
    let limit = cyberspine_frame::MAX_FRAME_LEN.saturating_sub(64);
    if message.len() <= limit {
        return;
    }

    let mut end = limit - SUFFIX.len();
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    message.truncate(end);
    message.push_str(SUFFIX);
}

/// A server the TCP loop can drive.
///
/// The implementation owns the reply buffer and returns a copy: the loop hands it
/// to a socket while the server may already be answering the next connection, and
/// borrowing the buffer across the lock would make that a data race.
pub trait Serve: Send + 'static {
    /// Answer exactly one request frame.
    fn reply(&mut self, connection: &mut Connection, request: &[u8]) -> Vec<u8>;

    /// The refusal to send when [`Serve::reply`] panicked.
    fn panic_reply(&mut self, message: &str) -> Vec<u8>;

    /// Answer one frame, keeping a panic inside it.
    ///
    /// A plugin is user code: a panic must cost one frame, not the mutex the
    /// transport holds. The refusal is inside the guard too, so a second panic
    /// degrades to an empty reply instead of escaping with the lock.
    fn answer_frame(&mut self, connection: &mut Connection, request: &[u8]) -> Vec<u8> {
        match catch_unwind(AssertUnwindSafe(|| self.reply(connection, request))) {
            Ok(reply) => reply,
            Err(payload) => {
                let message = panic_message(payload);
                catch_unwind(AssertUnwindSafe(|| self.panic_reply(&message))).unwrap_or_default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Connection, cap_message};

    #[test]
    fn a_retry_answers_only_after_the_reply_exists() {
        let mut connection = Connection::default();

        assert!(!connection.is_repeated(7), "the first frame is new");
        assert!(!connection.is_repeated(7), "nothing was committed yet");
        connection.commit(7);
        assert!(connection.is_repeated(7), "the retry is the same frame");
        assert!(!connection.is_repeated(8), "the next frame is new");
    }

    #[test]
    fn a_capped_message_keeps_its_prefix_and_names_the_cut() {
        let mut message = "x".repeat(cyberspine_frame::MAX_FRAME_LEN * 2);
        cap_message(&mut message);
        assert!(message.len() < cyberspine_frame::MAX_FRAME_LEN);
        assert!(message.ends_with("…(truncated)"));
    }

    #[test]
    fn a_short_message_is_left_alone() {
        let mut message = "the body refused".to_owned();
        cap_message(&mut message);
        assert_eq!(message, "the body refused");
    }
}
