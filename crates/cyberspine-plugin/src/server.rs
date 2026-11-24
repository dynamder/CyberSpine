//! State and plumbing every plugin server shares.
//!
//! The messages differ — a body answers `Sample`, an arbiter answers `Tick` —
//! but a server on either side is one frame in and one frame out, and both have
//! the same two shared concerns: what a *retry* means on one connection, and what
//! a socket-serving loop needs from the thing it drives.

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
    /// Whether this request was already executed. The reply it wants is still in
    /// the server's buffer.
    pub fn is_repeated(&mut self, seq: u64) -> bool {
        if self.last_seq == Some(seq) {
            return true;
        }
        self.last_seq = Some(seq);
        false
    }
}

/// A server the TCP loop can drive.
///
/// The implementation owns the reply buffer and returns a copy: the loop hands it
/// to a socket while the server may already be answering the next connection, and
/// borrowing the buffer across the lock would make that a data race.
pub trait Serve: Send + 'static {
    /// Answer exactly one request frame.
    fn answer_frame(&mut self, connection: &mut Connection, request: &[u8]) -> Vec<u8>;
}

#[cfg(test)]
mod tests {
    use super::Connection;

    #[test]
    fn a_retry_on_one_connection_is_a_lookup() {
        let mut connection = Connection::default();

        assert!(!connection.is_repeated(7), "the first frame is new");
        assert!(connection.is_repeated(7), "the retry is the same frame");
        assert!(!connection.is_repeated(8), "the next frame is new");
    }
}
