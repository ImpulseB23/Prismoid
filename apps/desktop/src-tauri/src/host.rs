//! Host-side helpers used by `lib.rs` to drive the sidecar: bootstrap
//! serialization, env-driven Twitch credentials, ring buffer batch parsing,
//! and the Windows handle inheritance toggles.
//!
//! The actual Tauri setup closure and the async drain loop live in `lib.rs`
//! because they are tightly coupled to the Tauri runtime and untestable
//! without a real app. Everything here is a pure function or a thin wrapper
//! around a platform API so the whole module stays unit-testable.

use std::time::Duration;

use serde::Serialize;

use crate::message::{parse_twitch_envelope, UnifiedMessage};
use crate::ringbuf::RawHandle;

/// Timeout for [`ringbuf::RingBufReader::wait_for_signal`] in the host drain
/// loop. In the happy path the sidecar signals the auto-reset event after
/// each ring write and the drain wakes immediately; this value only bounds
/// the worst-case latency of a missed signal.
///
/// 100 ms is a compromise between lost-signal recovery latency (still well
/// under the human-perceptible threshold) and idle CPU usage. An 8 ms timeout
/// would fire 125 times per second when the sidecar is quiet, burning wakes
/// for no reason; 100 ms fires 10 times per second, effectively free.
pub const SIGNAL_WAIT_TIMEOUT: Duration = Duration::from_millis(100);
pub const SIDECAR_BINARY: &str = "sidecar";

/// Twitch OAuth credentials sourced from environment variables for Phase 0 dev.
#[derive(Debug, Clone)]
pub struct TwitchCreds {
    pub client_id: String,
    pub access_token: String,
    pub broadcaster_id: String,
    pub user_id: String,
}

/// Parses a slice of raw ring-buffer payloads into [`UnifiedMessage`]s,
/// appending successful parses to the caller-owned `batch` scratch buffer.
/// The caller is responsible for clearing the scratch between drain ticks;
/// this function only appends.
///
/// Messages that fail to parse or that aren't chat notifications are dropped
/// with a log. Each parse is wrapped in `catch_unwind` so a panicking parser
/// cannot kill the drain loop (`docs/stability.md` §Rust Panic Handling).
pub fn parse_batch(raw: &[Vec<u8>], batch: &mut Vec<UnifiedMessage>) {
    for payload in raw {
        let slice = payload.as_slice();
        let outcome = std::panic::catch_unwind(|| parse_twitch_envelope(slice));
        match outcome {
            Ok(Ok(Some(msg))) => batch.push(msg),
            Ok(Ok(None)) => {}
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "parse failed, dropping message");
            }
            Err(_) => {
                tracing::error!("panic during envelope parse, dropping message");
            }
        }
    }
}

/// Serializes the bootstrap JSON line the Rust host writes to the sidecar's
/// stdin immediately after spawn. Includes the inheritable mapping HANDLE and
/// the auto-reset event HANDLE that the sidecar signals on each ring write.
pub fn build_bootstrap_line(
    handle: RawHandle,
    event_handle: RawHandle,
    size: usize,
) -> serde_json::Result<Vec<u8>> {
    #[derive(Serialize)]
    struct Bootstrap {
        shm_handle: u64,
        shm_event_handle: u64,
        shm_size: u64,
    }
    let payload = Bootstrap {
        shm_handle: handle as u64,
        shm_event_handle: event_handle as u64,
        shm_size: size as u64,
    };
    let mut bytes = serde_json::to_vec(&payload)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Serializes a `twitch_connect` control command line for the sidecar.
pub fn build_twitch_connect_line(creds: &TwitchCreds) -> serde_json::Result<Vec<u8>> {
    #[derive(Serialize)]
    struct ConnectCmd<'a> {
        cmd: &'a str,
        client_id: &'a str,
        token: &'a str,
        broadcaster_id: &'a str,
        user_id: &'a str,
    }
    let cmd = ConnectCmd {
        cmd: "twitch_connect",
        client_id: &creds.client_id,
        token: &creds.access_token,
        broadcaster_id: &creds.broadcaster_id,
        user_id: &creds.user_id,
    };
    let mut bytes = serde_json::to_vec(&cmd)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Marks a shared memory HANDLE inheritable just before spawning a child
/// process. See ADR 18 for why this is necessary.
#[cfg(windows)]
pub fn mark_handle_inheritable(handle: RawHandle) -> std::io::Result<()> {
    use windows::Win32::Foundation::{SetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT};
    unsafe {
        SetHandleInformation(
            HANDLE(handle as *mut _),
            HANDLE_FLAG_INHERIT.0,
            HANDLE_FLAG_INHERIT,
        )
        .map_err(std::io::Error::other)
    }
}

/// Clears the inheritable flag on a HANDLE immediately after the child is
/// spawned, so any subsequent child created by this process does not
/// accidentally inherit the same handle.
#[cfg(windows)]
pub fn unmark_handle_inheritable(handle: RawHandle) -> std::io::Result<()> {
    use windows::Win32::Foundation::{
        SetHandleInformation, HANDLE, HANDLE_FLAGS, HANDLE_FLAG_INHERIT,
    };
    unsafe {
        SetHandleInformation(
            HANDLE(handle as *mut _),
            HANDLE_FLAG_INHERIT.0,
            HANDLE_FLAGS(0),
        )
        .map_err(std::io::Error::other)
    }
}

#[cfg(not(windows))]
pub fn mark_handle_inheritable(_handle: RawHandle) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "handle inheritance not yet supported on this platform",
    ))
}

#[cfg(not(windows))]
pub fn unmark_handle_inheritable(_handle: RawHandle) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_line_has_expected_fields_and_newline() {
        let line = build_bootstrap_line(0xDEADBEEF, 0xCAFEBABE, 4096).unwrap();
        assert_eq!(line.last(), Some(&b'\n'));
        let body = &line[..line.len() - 1];
        let parsed: serde_json::Value = serde_json::from_slice(body).unwrap();
        assert_eq!(parsed["shm_handle"], 0xDEADBEEF_u64);
        assert_eq!(parsed["shm_event_handle"], 0xCAFEBABE_u64);
        assert_eq!(parsed["shm_size"], 4096_u64);
    }

    #[test]
    fn twitch_connect_line_has_all_required_fields() {
        let creds = TwitchCreds {
            client_id: "cid".into(),
            access_token: "tok".into(),
            broadcaster_id: "bid".into(),
            user_id: "uid".into(),
        };
        let line = build_twitch_connect_line(&creds).unwrap();
        assert_eq!(line.last(), Some(&b'\n'));
        let body = &line[..line.len() - 1];
        let parsed: serde_json::Value = serde_json::from_slice(body).unwrap();
        assert_eq!(parsed["cmd"], "twitch_connect");
        assert_eq!(parsed["client_id"], "cid");
        assert_eq!(parsed["token"], "tok");
        assert_eq!(parsed["broadcaster_id"], "bid");
        assert_eq!(parsed["user_id"], "uid");
    }

    #[test]
    fn parse_batch_filters_non_chat_and_parse_errors() {
        let viewer = br##"{
            "metadata": {"message_id":"m","message_type":"notification","message_timestamp":"2023-11-06T18:11:47.492Z"},
            "payload": {
                "subscription": {"type":"channel.chat.message"},
                "event": {
                    "chatter_user_id":"1","chatter_user_login":"u","chatter_user_name":"U",
                    "message_id":"mid","message":{"text":"hi"}
                }
            }
        }"##.to_vec();
        let keepalive = br##"{"metadata":{"message_id":"ka","message_type":"session_keepalive","message_timestamp":"2023-11-06T18:11:49.000Z"},"payload":{}}"##.to_vec();
        let junk = b"not json".to_vec();

        let raw = vec![viewer, keepalive, junk];
        let mut batch = Vec::new();
        parse_batch(&raw, &mut batch);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].message_text, "hi");
    }

    #[test]
    fn parse_batch_empty_input() {
        let mut batch = Vec::new();
        parse_batch(&[], &mut batch);
        assert!(batch.is_empty());
    }

    #[test]
    fn parse_batch_appends_to_existing_scratch() {
        // Verifies that parse_batch appends rather than clearing. The drain
        // loop owns clearing between ticks; this lets the loop avoid any
        // allocation churn on the hot path.
        let viewer = br##"{
            "metadata": {"message_id":"m","message_type":"notification","message_timestamp":"2023-11-06T18:11:47.492Z"},
            "payload": {
                "subscription": {"type":"channel.chat.message"},
                "event": {
                    "chatter_user_id":"1","chatter_user_login":"u","chatter_user_name":"U",
                    "message_id":"mid","message":{"text":"second"}
                }
            }
        }"##.to_vec();

        let mut batch = Vec::new();
        // Pretend a previous tick left one item in the scratch.
        parse_batch(std::slice::from_ref(&viewer), &mut batch);
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].message_text, "second");

        // Second call appends, scratch is NOT cleared.
        parse_batch(std::slice::from_ref(&viewer), &mut batch);
        assert_eq!(batch.len(), 2);
        assert_eq!(batch[1].message_text, "second");
    }

    #[cfg(windows)]
    #[test]
    fn mark_and_unmark_handle_inheritance_round_trip() {
        use crate::ringbuf;

        let reader = ringbuf::RingBufReader::create_owner(4096).unwrap();
        let handle = reader.raw_handle();

        mark_handle_inheritable(handle).expect("mark should succeed");
        unmark_handle_inheritable(handle).expect("unmark should succeed");
    }
}
