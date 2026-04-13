mod host;
mod message;
pub mod ringbuf;

// Re-exports for the bench harness. Gated so the public crate surface
// does not grow with bench-only plumbing in release builds.
#[cfg(any(test, feature = "__bench"))]
#[doc(hidden)]
pub use host::parse_batch;
#[cfg(any(test, feature = "__bench"))]
#[doc(hidden)]
pub use message::UnifiedMessage;

use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tracing_subscriber::EnvFilter;

use host::SIGNAL_WAIT_TIMEOUT;
use ringbuf::{RingBufReader, WaitOutcome, DEFAULT_CAPACITY};

#[tauri::command]
fn get_platform() -> &'static str {
    std::env::consts::OS
}

pub fn run() {
    // Phase 0 dev convenience: in debug builds only, load `.env.local` from
    // cwd or any parent before anything reads PRISMOID_TWITCH_*. Release
    // builds skip this entirely so a stray file in the install dir cannot
    // change runtime behavior. Real secret storage uses OS keychain via
    // OAuth, see docs/platform-apis.md §Twitch.
    #[cfg(debug_assertions)]
    match dotenvy::from_filename(".env.local") {
        Ok(_) => {}
        Err(dotenvy::Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => eprintln!("failed to load .env.local: {err}"),
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::from_default_env().add_directive("prismoid=debug".parse().unwrap()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![get_platform])
        .setup(setup)
        .run(tauri::generate_context!())
        .expect("failed to run prismoid");
}

/// Tauri setup hook. On Windows, creates the shm section + auto-reset event,
/// spawns the sidecar with both handles marked inheritable, bootstraps it,
/// and starts the drain task. On other platforms (not yet supported per
/// ADR 18), logs a warning and lets the Tauri app launch so frontend work
/// can proceed.
#[allow(clippy::unnecessary_wraps)]
fn setup<R: Runtime>(app: &mut tauri::App<R>) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        setup_sidecar(app)?;
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        tracing::warn!(
            "sidecar lifecycle is Windows-only for now; launching frontend without sidecar"
        );
    }
    Ok(())
}

#[cfg(windows)]
fn setup_sidecar<R: Runtime>(app: &mut tauri::App<R>) -> Result<(), Box<dyn std::error::Error>> {
    use host::{
        build_bootstrap_line, build_twitch_connect_line, mark_handle_inheritable,
        twitch_creds_from_env, unmark_handle_inheritable, SIDECAR_BINARY,
    };

    let reader = RingBufReader::create_owner(DEFAULT_CAPACITY)?;
    let handle = reader.raw_handle();
    let event_handle = reader
        .raw_event_handle()
        .expect("owner-created reader has event handle");
    let size = reader.map_size();

    mark_handle_inheritable(handle)?;
    if let Err(e) = mark_handle_inheritable(event_handle) {
        // Undo the mapping-handle mark before propagating, otherwise we leak
        // the inheritable flag on a partial failure.
        if let Err(undo) = unmark_handle_inheritable(handle) {
            tracing::error!(error = %undo, "failed to undo mapping handle mark after event mark failure");
        }
        return Err(e.into());
    }

    // RAII guard: the inheritable flag is cleared on any exit from this
    // function, including error paths. This keeps the window where the
    // HANDLEs are inheritable as narrow as possible (effectively: the sidecar
    // spawn). The Rust stdlib's CREATE_PROCESS_LOCK serializes our own child
    // spawns, but un-setting the flags immediately defends against any future
    // change where something else creates a process between mark and
    // function exit.
    struct InheritGuard {
        mapping: ringbuf::RawHandle,
        event: ringbuf::RawHandle,
    }
    impl Drop for InheritGuard {
        fn drop(&mut self) {
            if let Err(e) = unmark_handle_inheritable(self.mapping) {
                tracing::error!(error = %e, "failed to un-mark mapping handle inheritance in drop");
            }
            if let Err(e) = unmark_handle_inheritable(self.event) {
                tracing::error!(error = %e, "failed to un-mark event handle inheritance in drop");
            }
        }
    }
    let _inherit_guard = InheritGuard {
        mapping: handle,
        event: event_handle,
    };

    let sidecar = app.shell().sidecar(SIDECAR_BINARY)?;
    let (mut rx, mut child) = sidecar.spawn()?;

    // Spawn succeeded: the child has inherited both handles. Close the
    // inheritance window now rather than waiting for the function to return,
    // so that writing the bootstrap line, the twitch_connect command, and
    // spawning downstream async tasks all happen without the HANDLEs still
    // inheritable on this process. The guard's Drop still runs for error
    // paths that unwind before reaching this line.
    drop(_inherit_guard);

    let bootstrap_line = build_bootstrap_line(handle, event_handle, size)?;
    child.write(&bootstrap_line)?;
    tracing::info!("sidecar bootstrap written");

    if let Some(creds) = twitch_creds_from_env() {
        let connect_line = build_twitch_connect_line(&creds)?;
        child.write(&connect_line)?;
        tracing::info!(
            broadcaster = %creds.broadcaster_id,
            "sent twitch_connect with env creds"
        );
    } else {
        tracing::warn!("PRISMOID_TWITCH_* env vars not all set; launching without auto-connect");
    }

    // Drain the sidecar's CommandEvent stream for logging. This keeps
    // stdout/stderr from piling up and surfaces termination events.
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(bytes) => {
                    tracing::debug!(line = %String::from_utf8_lossy(&bytes), "sidecar stdout");
                }
                CommandEvent::Stderr(bytes) => {
                    tracing::debug!(line = %String::from_utf8_lossy(&bytes), "sidecar stderr");
                }
                CommandEvent::Terminated(payload) => {
                    tracing::warn!(code = ?payload.code, "sidecar terminated");
                    break;
                }
                _ => {}
            }
        }
    });

    let app_handle = app.app_handle().clone();
    // The drain loop blocks the calling thread on WaitForSingleObject, so it
    // lives on a dedicated blocking-pool thread rather than the tokio worker
    // pool. `spawn_blocking` tasks tolerate arbitrary blocking waits without
    // starving async tasks.
    tauri::async_runtime::spawn_blocking(move || run_drain_loop(reader, app_handle));

    Ok(())
}

/// Drain loop: parks on the auto-reset event signaled by the sidecar's writer
/// goroutine after each ring write. Wakes the instant new data lands, or at
/// the [`SIGNAL_WAIT_TIMEOUT`] as a belt-and-suspenders fallback for any
/// lost signal. Parses, batches, and emits once per wake.
///
/// The scratch `Vec<UnifiedMessage>` lives outside the loop and is cleared at
/// the top of each iteration, so steady-state operation allocates nothing on
/// the hot path (modulo the short-lived `Vec<Vec<u8>>` from `drain()` itself,
/// tracked for follow-up in PRI-8).
fn run_drain_loop<R: Runtime>(mut reader: RingBufReader, app: AppHandle<R>) {
    let timeout_ms: u32 = SIGNAL_WAIT_TIMEOUT
        .as_millis()
        .try_into()
        .expect("signal wait timeout fits in u32 ms");
    let mut batch: Vec<message::UnifiedMessage> = Vec::with_capacity(64);

    loop {
        match reader.wait_for_signal(timeout_ms) {
            Ok(WaitOutcome::Signaled) | Ok(WaitOutcome::TimedOut) => {}
            Err(e) => {
                tracing::error!(error = %e, "wait_for_signal failed, drain loop exiting");
                return;
            }
        }

        let raw = reader.drain();
        if raw.is_empty() {
            continue;
        }

        batch.clear();
        host::parse_batch(&raw, &mut batch);
        if batch.is_empty() {
            continue;
        }

        if let Err(e) = app.emit("chat_messages", &batch) {
            tracing::error!(error = %e, "failed to emit chat_messages");
        }
    }
}
