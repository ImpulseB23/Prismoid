//! Sidecar supervisor: owns the full lifecycle of the Go sidecar child
//! process. Spawns it, wires up the shared-memory ring, forwards stdout/stderr
//! to tracing, and respawns on termination with exponential backoff.
//!
//! The supervisor runs as a single long-lived tokio task kicked off from
//! `lib::setup`. Each iteration creates a fresh mapping + event pair
//! (handles are single-use by design, since the child closes them on exit)
//! and tears them down when the child dies. Emits `sidecar_status` events
//! on every state transition so the frontend can react without a second
//! refactor when UI chrome lands.
//!
//! Non-Windows targets are not supported yet; the supervisor is gated
//! behind `cfg(windows)` and callers fall back to a warn-and-bail in
//! `lib::setup`.

use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

#[cfg(windows)]
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
#[cfg(windows)]
use std::time::Instant;
#[cfg(windows)]
use tauri_plugin_shell::{
    process::{CommandChild, CommandEvent},
    ShellExt,
};

#[cfg(windows)]
use crate::emote_index::{EmoteBundle, EmoteIndex};
#[cfg(windows)]
use crate::host::{
    build_bootstrap_line, build_twitch_connect_line, mark_handle_inheritable, parse_batch,
    parse_sidecar_event, unmark_handle_inheritable, SidecarEvent, TwitchCreds, SIDECAR_BINARY,
    SIGNAL_WAIT_TIMEOUT,
};
#[cfg(windows)]
use crate::message::UnifiedMessage;
#[cfg(windows)]
use crate::ringbuf::{RawHandle, RingBufReader, WaitOutcome, DEFAULT_CAPACITY};
#[cfg(windows)]
use crate::sidecar_commands::SidecarCommandSender;
#[cfg(windows)]
use crate::twitch_auth::{AuthError, AuthManager, TWITCH_CLIENT_ID};
#[cfg(windows)]
use tokio::sync::Notify;

/// Supervisor timings. Defaults are production values; tests can override.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    /// A sidecar run whose uptime reaches this threshold is considered
    /// healthy, and the backoff resets to `initial_backoff` on its next
    /// termination. Without this the ladder would only ever ratchet up,
    /// even if the sidecar was stable for hours between crashes.
    pub healthy_threshold: Duration,
    /// Maximum gap between heartbeats from the sidecar before the
    /// supervisor declares the child unhealthy and force-kills it to
    /// trigger a respawn. `docs/stability.md` §Sidecar Health specifies
    /// 3 missed heartbeats at a 1 s interval, so the default is 3 s.
    pub heartbeat_timeout: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            initial_backoff: Duration::from_millis(250),
            max_backoff: Duration::from_secs(30),
            healthy_threshold: Duration::from_secs(60),
            heartbeat_timeout: Duration::from_secs(3),
        }
    }
}

/// Doubles the current backoff, clamped to `cfg.max_backoff`. Pure function
/// so the exponential ladder can be tested without spinning the runtime.
pub fn next_backoff(current: Duration, cfg: &SupervisorConfig) -> Duration {
    let doubled = current.saturating_mul(2);
    if doubled > cfg.max_backoff {
        cfg.max_backoff
    } else {
        doubled
    }
}

/// Emitted to the frontend on every supervisor state transition. The UI
/// ticket will listen for this; today nothing consumes it, but shipping
/// the event now means we don't have to touch the supervisor again.
#[derive(Debug, Clone, Serialize)]
pub struct SidecarStatus {
    pub state: &'static str,
    pub attempt: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backoff_ms: Option<u64>,
}

/// Kicks off the supervisor. Returns immediately; the supervisor runs on
/// a tauri async task until the app exits. The caller passes in the
/// shared [`AuthManager`] (also held by Tauri managed state for the
/// auth UI commands) and a `wakeup` notifier the supervisor awaits
/// while idle in `waiting_for_auth` so a successful sign-in starts the
/// sidecar within milliseconds instead of waiting out the 30 s poll.
#[cfg(windows)]
pub fn spawn<R: Runtime>(
    app: AppHandle<R>,
    auth: Arc<AuthManager>,
    wakeup: Arc<Notify>,
    sender: SidecarCommandSender,
) {
    let cfg = SupervisorConfig::default();
    tauri::async_runtime::spawn(async move {
        supervise(app, cfg, auth, wakeup, sender).await;
    });
}

#[cfg(windows)]
async fn supervise<R: Runtime>(
    app: AppHandle<R>,
    cfg: SupervisorConfig,
    auth: Arc<AuthManager>,
    wakeup: Arc<Notify>,
    sender: SidecarCommandSender,
) {
    // `client_id` lives in the shared AuthManager; broadcaster/user
    // identifiers ride inside the persisted [`TwitchTokens`] itself
    // (populated from the DCF response, stable across refresh).
    // Tokens live in the OS keychain, seeded via the SignIn overlay or
    // `cargo run --bin prismoid_dcf` and rotated automatically below
    // (ADR 29).
    let mut attempt: u32 = 0;
    let mut backoff = cfg.initial_backoff;

    loop {
        attempt += 1;
        emit_status(&app, "spawning", attempt, None);

        // Pull a fresh access token per iteration. Auto-refresh happens
        // inside load_or_refresh when we're within 5 min of expiry.
        let tokens = match auth.load_or_refresh().await {
            Ok(t) => t,
            Err(AuthError::NoTokens) | Err(AuthError::RefreshTokenInvalid) => {
                tracing::warn!(
                    "no valid Twitch tokens in keychain; click Sign in with Twitch in the app"
                );
                emit_status(&app, "waiting_for_auth", attempt, None);
                // Wait on the shared notifier (fired by
                // `twitch_complete_login` / `twitch_logout`) with a 30 s
                // floor so a process that boots before the keychain
                // service still recovers without a restart. Not a
                // respawn-pressure scenario, so we stay on a fixed
                // interval rather than the exponential ladder.
                let _ = tokio::time::timeout(Duration::from_secs(30), wakeup.notified()).await;
                continue;
            }
            Err(e) => {
                tracing::error!(error = %e, attempt, "token refresh failed; backing off");
                emit_status(&app, "backoff", attempt, Some(backoff.as_millis() as u64));
                tokio::time::sleep(backoff).await;
                backoff = next_backoff(backoff, &cfg);
                continue;
            }
        };

        // Single-account per ADR 30: the authenticated Twitch user IS
        // the broadcaster. `user_id` doubles as broadcaster_id in
        // EventSub's `channel.chat.message` subscription condition.
        let uid = tokens.user_id;
        let creds = TwitchCreds {
            client_id: TWITCH_CLIENT_ID.to_owned(),
            access_token: tokens.access_token,
            broadcaster_id: uid.clone(),
            user_id: uid,
        };

        let started = Instant::now();
        match run_once(&app, &cfg, attempt, Some(&creds), &sender).await {
            Ok(()) => tracing::info!(attempt, "sidecar iteration ended"),
            Err(e) => tracing::error!(error = %e, attempt, "sidecar iteration failed"),
        }
        // Always clear any lingering child handle once the iteration
        // ends, even on error paths above. Drops the CommandChild and
        // closes its stdin so the next iteration starts from a clean slate.
        let _ = sender.clear();

        if started.elapsed() >= cfg.healthy_threshold {
            backoff = cfg.initial_backoff;
            tracing::info!("sidecar run was healthy, backoff reset");
        }

        emit_status(&app, "backoff", attempt, Some(backoff.as_millis() as u64));
        tokio::time::sleep(backoff).await;
        backoff = next_backoff(backoff, &cfg);
    }
}

/// RAII wrapper that kills the wrapped sidecar on drop unless [`release`]
/// is called first. Used in [`run_once`] to avoid leaking children when a
/// post-spawn step (bootstrap serialize, stdin write, etc.) returns early
/// with `?`.
///
/// [`release`]: ChildGuard::release
#[cfg(windows)]
struct ChildGuard {
    inner: Option<CommandChild>,
}

#[cfg(windows)]
impl ChildGuard {
    fn new(child: CommandChild) -> Self {
        Self { inner: Some(child) }
    }

    /// Disarms the kill-on-drop and returns the wrapped child. Call this
    /// once the [`CommandEvent`] stream has taken over the child's
    /// lifecycle — every subsequent termination flows through
    /// [`CommandEvent::Terminated`], so an explicit kill is redundant.
    fn release(mut self) -> CommandChild {
        self.inner.take().expect("release called at most once")
    }
}

#[cfg(windows)]
impl std::ops::Deref for ChildGuard {
    type Target = CommandChild;
    fn deref(&self) -> &CommandChild {
        self.inner.as_ref().expect("child still owned by guard")
    }
}

#[cfg(windows)]
impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut CommandChild {
        self.inner.as_mut().expect("child still owned by guard")
    }
}

#[cfg(windows)]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(child) = self.inner.take() {
            if let Err(e) = child.kill() {
                tracing::error!(error = %e, "failed to kill sidecar on error path");
            }
        }
    }
}

#[cfg(windows)]
async fn run_once<R: Runtime>(
    app: &AppHandle<R>,
    cfg: &SupervisorConfig,
    attempt: u32,
    creds: Option<&TwitchCreds>,
    sender: &SidecarCommandSender,
) -> Result<(), Box<dyn std::error::Error>> {
    let reader = RingBufReader::create_owner(DEFAULT_CAPACITY)?;
    let handle = reader.raw_handle();
    let event_handle = reader
        .raw_event_handle()
        .expect("owner-created reader has event handle");
    let size = reader.map_size();

    mark_handle_inheritable(handle)?;
    if let Err(e) = mark_handle_inheritable(event_handle) {
        if let Err(undo) = unmark_handle_inheritable(handle) {
            tracing::error!(error = %undo, "failed to undo mapping mark after event mark failure");
        }
        return Err(e.into());
    }

    // RAII guard: clears the inheritable flag on both handles when dropped,
    // including on any unwinding error path between mark and explicit drop.
    // See ADR 18 and the Rust stdlib `CREATE_PROCESS_LOCK` comment for why
    // the window matters.
    struct InheritGuard {
        mapping: RawHandle,
        event: RawHandle,
    }
    impl Drop for InheritGuard {
        fn drop(&mut self) {
            if let Err(e) = unmark_handle_inheritable(self.mapping) {
                tracing::error!(error = %e, "failed to un-mark mapping inheritance");
            }
            if let Err(e) = unmark_handle_inheritable(self.event) {
                tracing::error!(error = %e, "failed to un-mark event inheritance");
            }
        }
    }
    let inherit_guard = InheritGuard {
        mapping: handle,
        event: event_handle,
    };

    let (mut rx, child) = app.shell().sidecar(SIDECAR_BINARY)?.spawn()?;
    // Spawn succeeded; child has inherited both handles. Clear the flags
    // now, not at scope exit, so any process we happen to spawn between
    // here and termination does not inherit the shared-memory handle.
    drop(inherit_guard);

    // Wrap the child so any `?` return between here and the event loop
    // kills it instead of leaking a zombie across respawn iterations.
    let mut child = ChildGuard::new(child);

    let bootstrap_line = build_bootstrap_line(handle, event_handle, size)?;
    child.write(&bootstrap_line)?;
    tracing::info!(attempt, "sidecar bootstrap written");

    if let Some(creds) = creds {
        let connect_line = build_twitch_connect_line(creds)?;
        child.write(&connect_line)?;
        tracing::info!(attempt, broadcaster = %creds.broadcaster_id, "sent twitch_connect");
    }

    // Disarm the kill-on-drop: the CommandEvent stream now owns the
    // child's lifecycle. Publish the released CommandChild into the
    // shared sender so Tauri commands can write control lines into its
    // stdin for the duration of the session. The heartbeat-timeout
    // branch below pulls it back out via `sender.clear()` for an
    // explicit kill, and the outer `supervise` loop also clears any
    // lingering handle once the iteration ends.
    sender.publish(child.release());

    // EmoteIndex lives for the lifetime of this sidecar run; a fresh one is
    // built on every respawn. Shared by the control-plane reader (which
    // swaps in fresh bundles on `emote_bundle`) and the drain loop (which
    // scans each parsed message against the current snapshot).
    let emote_index: Arc<EmoteIndex> = Arc::new(EmoteIndex::new());

    let shutdown = Arc::new(AtomicBool::new(false));
    let drain_shutdown = shutdown.clone();
    let drain_app = app.clone();
    let drain_index = emote_index.clone();
    let drain_handle = tauri::async_runtime::spawn_blocking(move || {
        run_drain_loop(reader, drain_app, drain_shutdown, drain_index);
    });

    emit_status(app, "running", attempt, None);

    run_event_loop(
        &mut rx,
        cfg.heartbeat_timeout,
        attempt,
        |bytes| handle_sidecar_stdout(bytes, app, &emote_index, sender),
        || {
            if let Some(c) = sender.clear() {
                if let Err(e) = c.kill() {
                    tracing::error!(error = %e, "kill after heartbeat timeout failed");
                }
            }
        },
        || emit_status(app, "unhealthy", attempt, None),
    )
    .await;

    // Release-store the shutdown flag; the drain loop does an Acquire-load
    // at the top of every iteration and exits after one final drain pass
    // so no pending messages are dropped on the floor.
    shutdown.store(true, Ordering::Release);
    if let Err(e) = drain_handle.await {
        tracing::error!(error = %e, "drain task join failed");
    }

    emit_status(app, "terminated", attempt, None);
    Ok(())
}

/// Drives the CommandEvent stream until the sidecar terminates or its
/// heartbeat stream goes silent for longer than `heartbeat_timeout`.
///
/// Extracted from [`run_once`] so the heartbeat/respawn logic can be
/// tested without a real Tauri runtime. Callers inject:
/// * `on_stdout`: parses each stdout batch and returns `true` if any
///   line was a heartbeat (resets the timeout deadline).
/// * `on_timeout_kill`: force-kills the child process. Called at most
///   once, the first time the heartbeat deadline elapses.
/// * `on_unhealthy`: emits the "unhealthy" sidecar status so the
///   frontend can render a respawn indicator.
///
/// After the first timeout fires the timeout branch is disabled and the
/// loop keeps draining events so the outer supervise loop doesn't race a
/// respawn against the dying child: we exit only on `Terminated` or when
/// the event stream closes on its own.
#[cfg(windows)]
async fn run_event_loop<F, K, U>(
    rx: &mut tauri::async_runtime::Receiver<CommandEvent>,
    heartbeat_timeout: Duration,
    attempt: u32,
    mut on_stdout: F,
    mut on_timeout_kill: K,
    mut on_unhealthy: U,
) where
    F: FnMut(&[u8]) -> bool,
    K: FnMut(),
    U: FnMut(),
{
    let mut last_heartbeat = Instant::now();
    let mut killed = false;
    loop {
        let deadline = last_heartbeat + heartbeat_timeout;
        tokio::select! {
            maybe_event = rx.recv() => {
                let Some(event) = maybe_event else { break };
                match event {
                    CommandEvent::Stdout(bytes) => {
                        if on_stdout(&bytes) {
                            last_heartbeat = Instant::now();
                        }
                    }
                    CommandEvent::Stderr(bytes) => {
                        tracing::debug!(line = %String::from_utf8_lossy(&bytes), "sidecar stderr");
                    }
                    CommandEvent::Error(msg) => {
                        // Transient stream error; the child may still be alive.
                        // Log and keep reading until we see Terminated (or the
                        // stream closes on its own).
                        tracing::error!(error = %msg, attempt, "sidecar command stream error");
                    }
                    CommandEvent::Terminated(payload) => {
                        tracing::warn!(code = ?payload.code, attempt, "sidecar terminated");
                        break;
                    }
                    // `CommandEvent` is `#[non_exhaustive]` upstream. Any future
                    // variant should at least surface at trace level instead of
                    // silently disappearing.
                    _ => {
                        tracing::trace!(attempt, "unhandled sidecar command event variant");
                    }
                }
            }
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)), if !killed => {
                let gap_ms = last_heartbeat.elapsed().as_millis() as u64;
                tracing::error!(
                    attempt,
                    gap_ms,
                    timeout_ms = heartbeat_timeout.as_millis() as u64,
                    "heartbeat timeout; killing sidecar to force respawn"
                );
                on_unhealthy();
                on_timeout_kill();
                killed = true;
            }
        }
    }
}

#[cfg(windows)]
fn run_drain_loop<R: Runtime>(
    mut reader: RingBufReader,
    app: AppHandle<R>,
    shutdown: Arc<AtomicBool>,
    emote_index: Arc<EmoteIndex>,
) {
    let timeout_ms: u32 = SIGNAL_WAIT_TIMEOUT
        .as_millis()
        .try_into()
        .expect("signal wait timeout fits in u32 ms");
    let mut batch: Vec<UnifiedMessage> = Vec::with_capacity(64);

    loop {
        if shutdown.load(Ordering::Acquire) {
            drain_and_emit(&mut reader, &app, &mut batch, &emote_index);
            return;
        }
        match reader.wait_for_signal(timeout_ms) {
            Ok(WaitOutcome::Signaled) | Ok(WaitOutcome::TimedOut) => {}
            Err(e) => {
                tracing::error!(error = %e, "wait_for_signal failed, drain loop exiting");
                return;
            }
        }
        drain_and_emit(&mut reader, &app, &mut batch, &emote_index);
    }
}

#[cfg(windows)]
fn drain_and_emit<R: Runtime>(
    reader: &mut RingBufReader,
    app: &AppHandle<R>,
    batch: &mut Vec<UnifiedMessage>,
    emote_index: &EmoteIndex,
) {
    let raw = reader.drain();
    if raw.is_empty() {
        return;
    }
    batch.clear();
    parse_batch(&raw, batch, emote_index);
    if batch.is_empty() {
        return;
    }
    if let Err(e) = app.emit("chat_messages", &*batch) {
        tracing::error!(error = %e, "failed to emit chat_messages");
    }
}

/// Processes one line of sidecar stdout. Tauri's shell plugin (in
/// non-raw mode, which is our default) emits one [`CommandEvent::Stdout`]
/// per `\n`-terminated line written by the child, with the trailing
/// newline stripped, so a partial line never reaches us. The split-on-`\n`
/// here is defensive: if the plugin ever changes that contract or the
/// sidecar buffers multiple JSON objects per write, each object still
/// parses independently. Empty pieces (trailing newline, blank lines) are
/// skipped.
///
/// Returns `true` if at least one heartbeat was observed in this batch,
/// which lets the supervisor reset the heartbeat-timeout deadline without
/// re-parsing each line.
#[cfg(windows)]
fn handle_sidecar_stdout<R: Runtime>(
    bytes: &[u8],
    app: &AppHandle<R>,
    emote_index: &Arc<EmoteIndex>,
    sender: &SidecarCommandSender,
) -> bool {
    scan_sidecar_stdout(
        bytes,
        |bundle| apply_emote_bundle(bundle, app, emote_index),
        |result| sender.complete_send_chat(result),
    )
}

/// Pure scan-and-dispatch core of [`handle_sidecar_stdout`]. Splits the
/// batch on newlines, parses each non-empty piece via
/// [`parse_sidecar_event`], returns `true` if any line was a heartbeat,
/// and invokes `on_bundle`/`on_send_result` for the corresponding
/// variants. Factored out so it can be unit-tested without a Tauri
/// runtime (the AppHandle-dependent work happens inside the closures).
fn scan_sidecar_stdout<F, G>(bytes: &[u8], mut on_bundle: F, mut on_send_result: G) -> bool
where
    F: FnMut(Box<crate::emote_index::EmoteBundle>),
    G: FnMut(crate::host::SendChatResult),
{
    let mut saw_heartbeat = false;
    for line in bytes.split(|b| *b == b'\n') {
        if line.is_empty() {
            continue;
        }
        match parse_sidecar_event(line) {
            SidecarEvent::Heartbeat => saw_heartbeat = true,
            SidecarEvent::EmoteBundle(bundle) => on_bundle(bundle),
            SidecarEvent::SendChatResult(r) => on_send_result(r),
            SidecarEvent::Other(t) => {
                tracing::debug!(msg_type = %t, "unhandled sidecar control message");
            }
            SidecarEvent::Invalid => {
                tracing::debug!(line = %String::from_utf8_lossy(line), "sidecar stdout (non-control)");
            }
        }
    }
    saw_heartbeat
}

/// Swaps a fresh [`EmoteBundle`] into the supervisor's [`EmoteIndex`] and
/// forwards a clone to the frontend. The frontend gets the full bundle
/// (emotes + badges + per-provider errors) so it can render emote and
/// badge images and surface partial-failure state without a second round
/// trip; only the emote sets feed [`EmoteIndex::load_bundle`], which is
/// lock-free for readers. Badges are resolved entirely on the frontend
/// from the same bundle — keeping the message hot path allocation-free.
#[cfg(windows)]
fn apply_emote_bundle<R: Runtime>(
    bundle: Box<EmoteBundle>,
    app: &AppHandle<R>,
    emote_index: &Arc<EmoteIndex>,
) {
    let total = bundle.total_emotes();
    // Frontend needs the full bundle to render URLs; cloning is cheap
    // compared to the network fetch that produced it and happens at most
    // once per channel join.
    if let Err(e) = app.emit("emote_bundle", bundle.as_ref()) {
        tracing::error!(error = %e, "failed to emit emote_bundle");
    }
    emote_index.load_bundle(*bundle);
    tracing::info!(total_emotes = total, "emote index rebuilt");
}

fn emit_status<R: Runtime>(
    app: &AppHandle<R>,
    state: &'static str,
    attempt: u32,
    backoff_ms: Option<u64>,
) {
    let status = SidecarStatus {
        state,
        attempt,
        backoff_ms,
    };
    if let Err(e) = app.emit("sidecar_status", &status) {
        tracing::warn!(error = %e, state, "failed to emit sidecar_status");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_matches_spec() {
        let cfg = SupervisorConfig::default();
        assert_eq!(cfg.initial_backoff, Duration::from_millis(250));
        assert_eq!(cfg.max_backoff, Duration::from_secs(30));
        assert_eq!(cfg.healthy_threshold, Duration::from_secs(60));
        // `docs/stability.md` §Sidecar Health: 3 missed heartbeats at
        // 1 s interval triggers respawn.
        assert_eq!(cfg.heartbeat_timeout, Duration::from_secs(3));
    }

    #[test]
    fn backoff_doubles_until_max() {
        let cfg = SupervisorConfig::default();
        let ladder = [
            (Duration::from_millis(250), Duration::from_millis(500)),
            (Duration::from_millis(500), Duration::from_secs(1)),
            (Duration::from_secs(1), Duration::from_secs(2)),
            (Duration::from_secs(2), Duration::from_secs(4)),
            (Duration::from_secs(4), Duration::from_secs(8)),
            (Duration::from_secs(8), Duration::from_secs(16)),
            (Duration::from_secs(16), Duration::from_secs(30)),
            (Duration::from_secs(30), Duration::from_secs(30)),
        ];
        for (input, expected) in ladder {
            assert_eq!(next_backoff(input, &cfg), expected, "input {input:?}");
        }
    }

    #[test]
    fn backoff_saturates_on_huge_input() {
        let cfg = SupervisorConfig::default();
        assert_eq!(next_backoff(Duration::MAX, &cfg), cfg.max_backoff);
    }

    #[test]
    fn sidecar_status_serializes_without_backoff_ms_when_none() {
        let status = SidecarStatus {
            state: "running",
            attempt: 3,
            backoff_ms: None,
        };
        let v: serde_json::Value = serde_json::to_value(&status).unwrap();
        assert_eq!(v["state"], "running");
        assert_eq!(v["attempt"], 3);
        assert!(v.get("backoff_ms").is_none());
    }

    #[test]
    fn sidecar_status_serializes_backoff_ms_when_some() {
        let status = SidecarStatus {
            state: "backoff",
            attempt: 7,
            backoff_ms: Some(4000),
        };
        let v: serde_json::Value = serde_json::to_value(&status).unwrap();
        assert_eq!(v["state"], "backoff");
        assert_eq!(v["backoff_ms"], 4000);
    }

    const VALID_EMOTE_BUNDLE_LINE: &[u8] = br#"{"type":"emote_bundle","payload":{"twitch_global_emotes":{"provider":"twitch","scope":"global","emotes":[{"id":"1","code":"Kappa","provider":"twitch","url_1x":"https://t/1"}]},"twitch_channel_emotes":{"provider":"twitch","scope":"channel","emotes":[]},"twitch_global_badges":{"scope":"global","badges":[]},"twitch_channel_badges":{"scope":"channel","badges":[]},"seventv_global":{"provider":"7tv","scope":"global","emotes":[]},"seventv_channel":{"provider":"7tv","scope":"channel","emotes":[]},"bttv_global":{"provider":"bttv","scope":"global","emotes":[]},"bttv_channel":{"provider":"bttv","scope":"channel","emotes":[]},"ffz_global":{"provider":"ffz","scope":"global","emotes":[]},"ffz_channel":{"provider":"ffz","scope":"channel","emotes":[]}}}"#;

    #[test]
    fn scan_sidecar_stdout_returns_false_on_empty_input() {
        assert!(!scan_sidecar_stdout(
            b"",
            |_| panic!("no bundle"),
            |_| panic!("no result")
        ));
        assert!(!scan_sidecar_stdout(
            b"\n\n\n",
            |_| panic!("no bundle"),
            |_| panic!("no result")
        ));
    }

    #[test]
    fn scan_sidecar_stdout_detects_single_heartbeat() {
        let line = br#"{"type":"heartbeat","payload":{"ts_ms":1,"counter":1}}"#;
        assert!(scan_sidecar_stdout(
            line,
            |_| panic!("no bundle"),
            |_| panic!("no result")
        ));
    }

    #[test]
    fn scan_sidecar_stdout_detects_heartbeat_in_multi_line_batch() {
        let mut batch = Vec::new();
        batch.extend_from_slice(b"garbage line\n");
        batch.extend_from_slice(br#"{"type":"future_thing","payload":{}}"#);
        batch.push(b'\n');
        batch.extend_from_slice(br#"{"type":"heartbeat","payload":{"ts_ms":1,"counter":1}}"#);
        batch.push(b'\n');
        assert!(scan_sidecar_stdout(
            &batch,
            |_| panic!("no bundle"),
            |_| panic!("no result")
        ));
    }

    #[test]
    fn scan_sidecar_stdout_reports_false_when_only_non_heartbeat_events() {
        let mut batch = Vec::new();
        batch.extend_from_slice(b"not json\n");
        batch.extend_from_slice(br#"{"type":"future_thing","payload":{}}"#);
        assert!(!scan_sidecar_stdout(
            &batch,
            |_| panic!("no bundle"),
            |_| panic!("no result")
        ));
    }

    #[test]
    fn scan_sidecar_stdout_dispatches_emote_bundle_and_tracks_heartbeat() {
        let mut batch = Vec::new();
        batch.extend_from_slice(VALID_EMOTE_BUNDLE_LINE);
        batch.push(b'\n');
        batch.extend_from_slice(br#"{"type":"heartbeat","payload":{"ts_ms":2,"counter":2}}"#);

        let mut bundles = 0_usize;
        let saw_heartbeat = scan_sidecar_stdout(
            &batch,
            |bundle| {
                assert_eq!(bundle.total_emotes(), 1);
                bundles += 1;
            },
            |_| panic!("no result"),
        );

        assert!(saw_heartbeat);
        assert_eq!(bundles, 1);
    }

    #[test]
    fn scan_sidecar_stdout_skips_empty_segments_between_newlines() {
        let mut batch = Vec::new();
        batch.extend_from_slice(b"\n\n");
        batch.extend_from_slice(br#"{"type":"heartbeat","payload":{"ts_ms":3,"counter":3}}"#);
        batch.extend_from_slice(b"\n\n");
        assert!(scan_sidecar_stdout(
            &batch,
            |_| panic!("no bundle"),
            |_| panic!("no result")
        ));
    }

    #[test]
    fn scan_sidecar_stdout_dispatches_send_chat_result() {
        let line = br#"{"type":"send_chat_result","payload":{"request_id":7,"ok":true,"message_id":"m1"}}"#;
        let mut got = None;
        let saw_heartbeat = scan_sidecar_stdout(line, |_| panic!("no bundle"), |r| got = Some(r));
        assert!(!saw_heartbeat);
        let r = got.expect("result dispatched");
        assert_eq!(r.request_id, 7);
        assert!(r.ok);
        assert_eq!(r.message_id, "m1");
    }

    // -- run_event_loop tests --------------------------------------------
    //
    // These drive the real event loop with an mpsc channel of
    // `CommandEvent`s under `tokio::time::pause()`, letting us assert the
    // heartbeat-respawn behaviour deterministically without a Tauri
    // runtime or a real child process.

    use std::cell::Cell;
    use std::rc::Rc;
    use tauri_plugin_shell::process::TerminatedPayload;
    use tokio::sync::mpsc;

    const HB_LINE: &[u8] = br#"{"type":"heartbeat","payload":{"ts_ms":1,"counter":1}}"#;

    #[derive(Default)]
    struct LoopProbe {
        stdout_calls: Rc<Cell<u32>>,
        kill_calls: Rc<Cell<u32>>,
        unhealthy_calls: Rc<Cell<u32>>,
    }

    impl LoopProbe {
        fn stdout(&self) -> u32 {
            self.stdout_calls.get()
        }
        fn kills(&self) -> u32 {
            self.kill_calls.get()
        }
        fn unhealthy(&self) -> u32 {
            self.unhealthy_calls.get()
        }
    }

    #[tokio::test(start_paused = true)]
    async fn run_event_loop_exits_on_terminated_event() {
        let (tx, mut rx) = mpsc::channel::<CommandEvent>(4);
        let probe = LoopProbe::default();
        let stdout_calls = probe.stdout_calls.clone();
        let kill_calls = probe.kill_calls.clone();
        let unhealthy_calls = probe.unhealthy_calls.clone();

        tx.send(CommandEvent::Terminated(TerminatedPayload {
            code: Some(0),
            signal: None,
        }))
        .await
        .unwrap();

        run_event_loop(
            &mut rx,
            Duration::from_secs(60),
            1,
            |_| {
                stdout_calls.set(stdout_calls.get() + 1);
                false
            },
            || kill_calls.set(kill_calls.get() + 1),
            || unhealthy_calls.set(unhealthy_calls.get() + 1),
        )
        .await;

        assert_eq!(probe.stdout(), 0);
        assert_eq!(probe.kills(), 0);
        assert_eq!(probe.unhealthy(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn run_event_loop_exits_when_stream_closes() {
        let (tx, mut rx) = mpsc::channel::<CommandEvent>(4);
        drop(tx); // close the channel immediately
        let probe = LoopProbe::default();
        let kill_calls = probe.kill_calls.clone();
        let unhealthy_calls = probe.unhealthy_calls.clone();

        run_event_loop(
            &mut rx,
            Duration::from_secs(60),
            1,
            |_| false,
            || kill_calls.set(kill_calls.get() + 1),
            || unhealthy_calls.set(unhealthy_calls.get() + 1),
        )
        .await;

        assert_eq!(probe.kills(), 0);
        assert_eq!(probe.unhealthy(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn run_event_loop_kills_on_heartbeat_timeout() {
        let (tx, mut rx) = mpsc::channel::<CommandEvent>(4);
        let probe = LoopProbe::default();
        let kill_calls = probe.kill_calls.clone();
        let unhealthy_calls = probe.unhealthy_calls.clone();

        // Drive the loop and feed a Terminated after the kill fires so it
        // exits cleanly. spawn_local isn't available on a single-threaded
        // runtime without LocalSet, so send the event synchronously after
        // `tokio::time::advance`.
        let timeout = Duration::from_secs(3);
        let loop_fut = run_event_loop(
            &mut rx,
            timeout,
            1,
            |_| false,
            || kill_calls.set(kill_calls.get() + 1),
            || unhealthy_calls.set(unhealthy_calls.get() + 1),
        );

        tokio::pin!(loop_fut);

        // Advance past the heartbeat deadline; the select! should pick the
        // timeout branch.
        tokio::select! {
            _ = &mut loop_fut => panic!("loop exited before timeout"),
            _ = tokio::time::sleep(timeout + Duration::from_millis(100)) => {}
        }

        // Now close the stream so the loop exits.
        drop(tx);
        loop_fut.await;

        assert_eq!(probe.kills(), 1);
        assert_eq!(probe.unhealthy(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn run_event_loop_heartbeat_resets_deadline() {
        let (tx, mut rx) = mpsc::channel::<CommandEvent>(16);
        let probe = LoopProbe::default();
        let stdout_calls = probe.stdout_calls.clone();
        let kill_calls = probe.kill_calls.clone();

        let timeout = Duration::from_secs(3);

        // Pre-load two heartbeat batches, each followed by a ~2s pause.
        // After the second heartbeat we send Terminated so the loop exits
        // without the timeout branch ever firing.
        tx.send(CommandEvent::Stdout(HB_LINE.to_vec()))
            .await
            .unwrap();
        tx.send(CommandEvent::Stdout(HB_LINE.to_vec()))
            .await
            .unwrap();
        tx.send(CommandEvent::Terminated(TerminatedPayload {
            code: Some(0),
            signal: None,
        }))
        .await
        .unwrap();

        run_event_loop(
            &mut rx,
            timeout,
            1,
            |bytes| {
                stdout_calls.set(stdout_calls.get() + 1);
                // Forward to the pure scanner so heartbeat detection is real.
                scan_sidecar_stdout(bytes, |_| {}, |_| {})
            },
            || kill_calls.set(kill_calls.get() + 1),
            || {},
        )
        .await;

        assert_eq!(probe.stdout(), 2);
        assert_eq!(probe.kills(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn run_event_loop_logs_stderr_and_error_without_affecting_state() {
        let (tx, mut rx) = mpsc::channel::<CommandEvent>(8);
        let probe = LoopProbe::default();
        let kill_calls = probe.kill_calls.clone();

        tx.send(CommandEvent::Stderr(b"some warning\n".to_vec()))
            .await
            .unwrap();
        tx.send(CommandEvent::Error("stream hiccup".into()))
            .await
            .unwrap();
        tx.send(CommandEvent::Terminated(TerminatedPayload {
            code: Some(1),
            signal: None,
        }))
        .await
        .unwrap();

        run_event_loop(
            &mut rx,
            Duration::from_secs(60),
            7,
            |_| false,
            || kill_calls.set(kill_calls.get() + 1),
            || {},
        )
        .await;

        assert_eq!(probe.kills(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn run_event_loop_kills_at_most_once_across_multiple_deadlines() {
        let (tx, mut rx) = mpsc::channel::<CommandEvent>(4);
        let probe = LoopProbe::default();
        let kill_calls = probe.kill_calls.clone();
        let unhealthy_calls = probe.unhealthy_calls.clone();

        let timeout = Duration::from_millis(500);
        let loop_fut = run_event_loop(
            &mut rx,
            timeout,
            1,
            |_| false,
            || kill_calls.set(kill_calls.get() + 1),
            || unhealthy_calls.set(unhealthy_calls.get() + 1),
        );
        tokio::pin!(loop_fut);

        // Let the timeout branch fire once.
        tokio::select! {
            _ = &mut loop_fut => panic!("loop exited before timeout"),
            _ = tokio::time::sleep(timeout * 2) => {}
        }

        // Advance another few timeouts with the stream still open; the
        // timeout branch is gated on `!killed` so it must not fire again.
        tokio::select! {
            _ = &mut loop_fut => panic!("loop exited unexpectedly"),
            _ = tokio::time::sleep(timeout * 5) => {}
        }

        drop(tx);
        loop_fut.await;

        assert_eq!(probe.kills(), 1);
        assert_eq!(probe.unhealthy(), 1);
    }
}
