//! Tauri commands that send control-plane messages to the running Go
//! sidecar over its stdin and await structured responses on its stdout.
//!
//! The sender is a clone-able handle around an `Arc<Mutex<Inner>>` that
//! owns:
//!   * the live [`tauri_plugin_shell::process::CommandChild`] (so commands
//!     can write into its stdin pipe), and
//!   * a map of in-flight request ids → oneshot completers (so the
//!     supervisor can route a `send_chat_result` notification back to the
//!     awaiting Tauri invocation).
//!
//! The supervisor publishes the child after a successful spawn + bootstrap
//! and clears it on termination. `clear` also drops every pending
//! completer, which fails the awaiting commands with
//! [`SendCommandError::SidecarNotRunning`] instead of leaving them
//! hanging forever.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use serde::Serialize;
use tauri::State;
use tokio::sync::oneshot;

#[cfg(windows)]
use tauri_plugin_shell::process::CommandChild;

use crate::host::{build_send_chat_message_line, SendChatMessageArgs, SendChatResult};
use crate::twitch_auth::{AuthError, AuthState, TWITCH_CLIENT_ID};

/// Inner state shared between the supervisor (publish/clear), the
/// command (write/register), and the stdout dispatcher (complete).
struct Inner {
    #[cfg(windows)]
    child: Option<CommandChild>,
    pending: HashMap<u64, oneshot::Sender<SendChatResult>>,
    next_id: u64,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            #[cfg(windows)]
            child: None,
            pending: HashMap::new(),
            // Start at 1 so a serialized 0 (which the Go side may omit
            // because of `omitempty`) can never be confused with a real id.
            next_id: 1,
        }
    }
}

/// Shared handle the supervisor uses to publish the live sidecar child
/// and that command handlers use to write control lines into its stdin.
#[derive(Default, Clone)]
pub struct SidecarCommandSender {
    inner: Arc<Mutex<Inner>>,
}

/// Recovers the inner state from a poisoned mutex. A poison just means a
/// previous holder panicked; the data is still consistent (we only ever
/// hold the lock for short, infallible operations) so taking it is safe
/// and lets us continue serving commands rather than crashing the app.
fn unpoison<'a, T>(
    result: Result<MutexGuard<'a, T>, PoisonError<MutexGuard<'a, T>>>,
) -> MutexGuard<'a, T> {
    result.unwrap_or_else(|e| {
        tracing::warn!("sidecar sender mutex was poisoned; recovering");
        e.into_inner()
    })
}

impl SidecarCommandSender {
    /// Publishes the live child. Called by the supervisor right after
    /// the bootstrap + initial connect lines have been written so the
    /// child is fully ready to accept commands. Replaces any previous
    /// child handle (e.g. carried over from a respawn) and drops it,
    /// which closes the prior stdin pipe.
    #[cfg(windows)]
    pub fn publish(&self, child: CommandChild) {
        let mut g = unpoison(self.inner.lock());
        g.child = Some(child);
    }

    /// Clears the child handle and drops every pending completer so the
    /// awaiting commands resolve with [`SendCommandError::SidecarNotRunning`]
    /// instead of waiting forever for a response that will never come.
    #[cfg(windows)]
    pub fn clear(&self) -> Option<CommandChild> {
        let mut g = unpoison(self.inner.lock());
        g.pending.clear();
        g.child.take()
    }

    /// On non-Windows builds clearing only drops pending completers;
    /// there is no child handle to return.
    #[cfg(not(windows))]
    pub fn clear(&self) {
        let mut g = unpoison(self.inner.lock());
        g.pending.clear();
    }

    /// Routes a `send_chat_result` notification from the sidecar's
    /// stdout to the awaiting command. A no-op if no completer is
    /// registered for the id (e.g. the awaiting future was dropped).
    pub fn complete_send_chat(&self, result: SendChatResult) {
        let mut g = unpoison(self.inner.lock());
        if let Some(tx) = g.pending.remove(&result.request_id) {
            let _: Result<(), _> = tx.send(result);
        }
    }

    /// Allocates a fresh request id, registers the oneshot sender under
    /// it, and writes the given control line to the child's stdin in a
    /// single locked section so the line and the registration can't race
    /// against a concurrent `clear`.
    #[cfg(windows)]
    fn send_with_pending<F>(
        &self,
        tx: oneshot::Sender<SendChatResult>,
        build_line: F,
    ) -> Result<(), SendCommandError>
    where
        F: FnOnce(u64) -> Result<Vec<u8>, serde_json::Error>,
    {
        let mut g = unpoison(self.inner.lock());
        if g.child.is_none() {
            return Err(SendCommandError::SidecarNotRunning);
        }
        let id = g.next_id;
        let line = build_line(id).map_err(|e| SendCommandError::Json {
            message: e.to_string(),
        })?;
        let child = g
            .child
            .as_mut()
            .ok_or(SendCommandError::SidecarNotRunning)?;
        child
            .write(&line)
            .map_err(|e: tauri_plugin_shell::Error| SendCommandError::Io {
                message: e.to_string(),
            })?;
        // Bump and skip 0 on wraparound.
        g.next_id = match g.next_id.wrapping_add(1) {
            0 => 1,
            n => n,
        };
        g.pending.insert(id, tx);
        Ok(())
    }

    #[cfg(not(windows))]
    fn send_with_pending<F>(
        &self,
        _tx: oneshot::Sender<SendChatResult>,
        _build_line: F,
    ) -> Result<(), SendCommandError>
    where
        F: FnOnce(u64) -> Result<Vec<u8>, serde_json::Error>,
    {
        Err(SendCommandError::SidecarNotRunning)
    }
}

/// Frontend-facing error for `twitch_send_message` and any future
/// command. `kind` is a stable string the UI matches against.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SendCommandError {
    NotLoggedIn {
        message: String,
    },
    EmptyMessage,
    MessageTooLong {
        max_bytes: usize,
    },
    SidecarNotRunning,
    Io {
        message: String,
    },
    Auth {
        message: String,
    },
    Json {
        message: String,
    },
    /// Twitch accepted the request but rejected the message (drop reason)
    /// or returned a non-2xx response. `code` is the Helix drop-reason
    /// tag (empty for transport-level errors).
    Helix {
        code: String,
        message: String,
    },
}

impl SendCommandError {
    fn auth(err: AuthError) -> Self {
        match err {
            AuthError::NoTokens | AuthError::RefreshTokenInvalid => Self::NotLoggedIn {
                message: err.to_string(),
            },
            other => Self::Auth {
                message: other.to_string(),
            },
        }
    }

    fn from_send_result(r: SendChatResult) -> Result<(), Self> {
        if r.ok {
            return Ok(());
        }
        if !r.drop_code.is_empty() || !r.drop_message.is_empty() {
            return Err(Self::Helix {
                code: r.drop_code,
                message: if r.drop_message.is_empty() {
                    "message rejected".to_string()
                } else {
                    r.drop_message
                },
            });
        }
        Err(Self::Helix {
            code: String::new(),
            message: if r.error_message.is_empty() {
                "send failed".to_string()
            } else {
                r.error_message
            },
        })
    }
}

/// Maximum chat message length accepted by Twitch Helix POST
/// /chat/messages. Mirrored on the Rust side so we reject oversized
/// payloads before they cross the IPC boundary.
pub const MAX_CHAT_MESSAGE_BYTES: usize = 500;

#[tauri::command]
pub async fn twitch_send_message(
    auth: State<'_, AuthState>,
    sender: State<'_, SidecarCommandSender>,
    text: String,
) -> Result<(), SendCommandError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(SendCommandError::EmptyMessage);
    }
    if trimmed.len() > MAX_CHAT_MESSAGE_BYTES {
        return Err(SendCommandError::MessageTooLong {
            max_bytes: MAX_CHAT_MESSAGE_BYTES,
        });
    }

    let tokens = auth
        .manager
        .load_or_refresh()
        .await
        .map_err(SendCommandError::auth)?;

    let (tx, rx) = oneshot::channel();
    sender.send_with_pending(tx, |request_id| {
        build_send_chat_message_line(SendChatMessageArgs {
            client_id: TWITCH_CLIENT_ID,
            access_token: &tokens.access_token,
            broadcaster_id: &tokens.user_id,
            user_id: &tokens.user_id,
            message: trimmed,
            request_id,
        })
    })?;

    // Sender dropped (sidecar terminated, completer cleared) → treat as
    // not-running rather than leaking the await.
    let result = rx.await.map_err(|_| SendCommandError::SidecarNotRunning)?;
    SendCommandError::from_send_result(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_mapping_no_tokens_is_not_logged_in() {
        let mapped = SendCommandError::auth(AuthError::NoTokens);
        assert!(matches!(mapped, SendCommandError::NotLoggedIn { .. }));
    }

    #[test]
    fn auth_mapping_refresh_invalid_is_not_logged_in() {
        let mapped = SendCommandError::auth(AuthError::RefreshTokenInvalid);
        assert!(matches!(mapped, SendCommandError::NotLoggedIn { .. }));
    }

    #[test]
    fn auth_mapping_other_is_auth() {
        let mapped = SendCommandError::auth(AuthError::OAuth("boom".into()));
        assert!(matches!(mapped, SendCommandError::Auth { .. }));
    }

    fn make_result(ok: bool, drop_code: &str, drop_message: &str, error: &str) -> SendChatResult {
        SendChatResult {
            request_id: 1,
            ok,
            message_id: if ok { "abc".into() } else { String::new() },
            drop_code: drop_code.into(),
            drop_message: drop_message.into(),
            error_message: error.into(),
        }
    }

    #[test]
    fn from_send_result_ok_is_ok() {
        assert!(SendCommandError::from_send_result(make_result(true, "", "", "")).is_ok());
    }

    #[test]
    fn from_send_result_drop_maps_to_helix() {
        match SendCommandError::from_send_result(make_result(
            false,
            "msg_duplicate",
            "duplicate",
            "",
        ))
        .unwrap_err()
        {
            SendCommandError::Helix { code, message } => {
                assert_eq!(code, "msg_duplicate");
                assert_eq!(message, "duplicate");
            }
            other => panic!("expected Helix, got {other:?}"),
        }
    }

    #[test]
    fn from_send_result_error_only_maps_to_helix() {
        match SendCommandError::from_send_result(make_result(false, "", "", "401")).unwrap_err() {
            SendCommandError::Helix { code, message } => {
                assert!(code.is_empty());
                assert_eq!(message, "401");
            }
            other => panic!("expected Helix, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_without_child_returns_not_running() {
        let sender = SidecarCommandSender::default();
        let (tx, _rx) = oneshot::channel();
        let err = sender
            .send_with_pending(tx, |_id| Ok(b"x\n".to_vec()))
            .expect_err("must error");
        assert!(matches!(err, SendCommandError::SidecarNotRunning));
    }

    #[test]
    fn complete_send_chat_no_pending_is_noop() {
        // No registration, no panic. Idempotent so a stray late
        // notification can't blow up the supervisor's stdout loop.
        let sender = SidecarCommandSender::default();
        sender.complete_send_chat(make_result(true, "", "", ""));
    }

    #[test]
    fn clear_drops_pending_completers() {
        let sender = SidecarCommandSender::default();
        let (tx, mut rx) = oneshot::channel::<SendChatResult>();
        {
            let mut g = unpoison(sender.inner.lock());
            g.pending.insert(7, tx);
        }
        let _ = sender.clear();
        match rx.try_recv() {
            Err(oneshot::error::TryRecvError::Closed) => {}
            other => panic!("expected Closed, got {other:?}"),
        }
    }

    #[test]
    fn complete_routes_to_pending_completer() {
        let sender = SidecarCommandSender::default();
        let (tx, mut rx) = oneshot::channel::<SendChatResult>();
        {
            let mut g = unpoison(sender.inner.lock());
            g.pending.insert(42, tx);
        }
        let mut r = make_result(true, "", "", "");
        r.request_id = 42;
        sender.complete_send_chat(r);
        let got = rx.try_recv().expect("should have received");
        assert_eq!(got.request_id, 42);
        assert!(got.ok);
    }
}
