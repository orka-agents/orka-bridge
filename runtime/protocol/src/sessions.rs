//! Read-only Orka Sessions for Overview. The guest reads them with its own identity and
//! returns an `OrkaReply` whose body is a bounded projection, never the raw Session.
//!
//! A list page body is `{"items":[Session…],"metadata":{"continue":String|null}}`, with
//! `"cut":true` in `metadata` when Orka's next cursor was too long to pass on; each Session
//! keeps only `name`, `namespace`, `sessionType`, `messageCount`, `createdAt`,
//! `updatedAt` and `activeTask`.
//!
//! A Session body is `{"name","namespace","transcript","transcriptCut"}`. `transcript` holds
//! the latest messages as JSONL lines of `role`, `content`, `cut`, `toolCalls` (names only)
//! and `ts`; `transcriptCut` is true when older lines were left out.

/// Sessions per list page the guest asks Orka for.
pub const SESSION_PAGE: usize = 20;
/// Largest Orka reply the guest reads for a Sessions action, before projection.
pub const MAX_SESSION_READ: usize = 8 * 1024 * 1024;
/// Latest transcript messages a Session body keeps.
pub const SESSION_MESSAGES: usize = 200;
/// Bytes of each message's text a Session body keeps.
pub const SESSION_MESSAGE_TEXT: usize = 16 * 1024;
/// Longest list cursor accepted from Orka or the host.
pub const MAX_SESSION_CURSOR: usize = 1024;

/// A Session name that may go in a request path: Orka's names are object-like.
pub fn valid_session_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 253
        && name.as_bytes()[0].is_ascii_alphanumeric()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// An opaque Orka list cursor: printable, URL-safe-ish ASCII, bounded.
pub fn valid_session_cursor(cursor: &str) -> bool {
    !cursor.is_empty()
        && cursor.len() <= MAX_SESSION_CURSOR
        && cursor.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'=' | b'+' | b'/' | b':')
        })
}
