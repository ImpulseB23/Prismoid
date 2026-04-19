//! Tauri commands that send control-plane messages to the running Go
//! sidecar over its stdin. Decoupled from the supervisor so the command
//! handlers can be tested without spinning a real child process.
//!
//! The sender is a clone-able handle around an `Arc<Mutex<Option<...>>>`
//! holding the live [`tauri_plugin_shell::process::CommandChild`]. The
//! supervisor publishes the child after a successful spawn + bootstrap
//! and clears it on termination (or never publishes it on platforms where
//! the sidecar isn't supported). Commands fail fast with a structured
//! error when no child is alive instead of blocking on a vanished pipe.

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::State;

#[cfg(windows)]
use tauri_plugin_shell::process::CommandChild;

use crate::host::{build_send_chat_message_line, SendChatMessageArgs};
use crate::twitch_auth::{AuthError, AuthState, TWITCH_CLIENT_ID};

/// Shared handle the supervisor uses to publish the live sidecar child
/// and that command handlers use to write control lines into its stdin.
/// On non-Windows builds the inner type degrades to `()` so the
/// supervisor's call sites compile without `#[cfg]` everywhere; commands
/// always return [`SendCommandError::SidecarNotRunning`] there.
#[derive(Default, Clone)]
pub struct SidecarCommandSender {
    #[cfg(windows)]
    inner: Arc<Mutex<Option<CommandChild>>>,
    #[cfg(not(windows))]
    inner: Arc<Mutex<()>>,
}

impl SidecarCommandSender {
    /// Publishes the live child. Called by the supervisor right after
    /// the bootstrap + initial connect lines have been written so the
    /// child is fully ready to accept commands. Replaces any previous
    /// child handle (e.g. carried over from a respawn) and drops it,
    /// which closes the prior stdin pipe.
    #[cfg(windows)]
    pub fn publish(&self, child: CommandChild) {
        *self.inner.lock().expect("sidecar sender mutex poisoned") = Some(child);
    }

    /// Clears the child handle. Called by the supervisor when the child
    /// terminates or when the heartbeat-timeout path needs to take
    /// ownership for an explicit `kill`.
    #[cfg(windows)]
    pub fn clear(&self) -> Option<CommandChild> {
        self.inner
            .lock()
            .expect("sidecar sender mutex poisoned")
            .take()
    }

    /// Writes a single newline-terminated command line to the child's
    /// stdin. Errors propagate from the underlying pipe write so callers
    /// can map them to user-facing failures.
    #[cfg(windows)]
    fn write_line(&self, bytes: &[u8]) -> Result<(), SendCommandError> {
        let mut guard = self.inner.lock().expect("sidecar sender mutex poisoned");
        let child = guard.as_mut().ok_or(SendCommandError::SidecarNotRunning)?;
        child.write(bytes).map_err(|e| SendCommandError::Io {
            message: e.to_string(),
        })
    }

    #[cfg(not(windows))]
    fn write_line(&self, _bytes: &[u8]) -> Result<(), SendCommandError> {
        Err(SendCommandError::SidecarNotRunning)
    }
}

/// Frontend-facing error for `twitch_send_message` and any future
/// command. `kind` is a stable string the UI matches against; `message`
/// is a human-readable diagnostic.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SendCommandError {
    NotLoggedIn { message: String },
    EmptyMessage,
    MessageTooLong { max_bytes: usize },
    SidecarNotRunning,
    Io { message: String },
    Auth { message: String },
    Json { message: String },
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

    let line = build_send_chat_message_line(SendChatMessageArgs {
        client_id: TWITCH_CLIENT_ID,
        access_token: &tokens.access_token,
        broadcaster_id: &tokens.user_id,
        user_id: &tokens.user_id,
        message: trimmed,
    })
    .map_err(|e| SendCommandError::Json {
        message: e.to_string(),
    })?;

    sender.write_line(&line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_mapping_no_tokens_is_not_logged_in() {
        let mapped = SendCommandError::auth(AuthError::NoTokens);
        match mapped {
            SendCommandError::NotLoggedIn { .. } => {}
            other => panic!("expected NotLoggedIn, got {other:?}"),
        }
    }

    #[test]
    fn auth_mapping_refresh_invalid_is_not_logged_in() {
        let mapped = SendCommandError::auth(AuthError::RefreshTokenInvalid);
        match mapped {
            SendCommandError::NotLoggedIn { .. } => {}
            other => panic!("expected NotLoggedIn, got {other:?}"),
        }
    }

    #[test]
    fn auth_mapping_other_is_auth() {
        let mapped = SendCommandError::auth(AuthError::OAuth("boom".into()));
        match mapped {
            SendCommandError::Auth { .. } => {}
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_without_child_returns_not_running() {
        let sender = SidecarCommandSender::default();
        let err = sender.write_line(b"x\n").expect_err("must error");
        assert!(matches!(err, SendCommandError::SidecarNotRunning));
    }
}
