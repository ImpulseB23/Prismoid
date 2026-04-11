mod host;
mod message;
pub mod ringbuf;

use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_shell::process::CommandEvent;
use tauri_plugin_shell::ShellExt;
use tracing_subscriber::EnvFilter;

use host::{parse_batch, DRAIN_INTERVAL};
use ringbuf::{RingBufReader, DEFAULT_CAPACITY};

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

/// Tauri setup hook. On Windows, creates the shm section, spawns the sidecar
/// with the HANDLE marked inheritable, bootstraps it, and starts the drain
/// task. On other platforms (not yet supported per ADR 18), logs a warning
/// and lets the Tauri app launch so frontend work can proceed.
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
    let size = reader.map_size();

    mark_handle_inheritable(handle)?;

    // RAII guard: the inheritable flag is cleared on any exit from this
    // function, including error paths. This keeps the window where the HANDLE
    // is inheritable as narrow as possible (effectively: the sidecar spawn).
    // The Rust stdlib's CREATE_PROCESS_LOCK serializes our own child spawns,
    // but un-setting the flag immediately defends against any future change
    // where something else creates a process between mark and function exit.
    struct InheritGuard(ringbuf::RawHandle);
    impl Drop for InheritGuard {
        fn drop(&mut self) {
            if let Err(e) = unmark_handle_inheritable(self.0) {
                tracing::error!(error = %e, "failed to un-mark handle inheritance in drop");
            }
        }
    }
    let _inherit_guard = InheritGuard(handle);

    let sidecar = app.shell().sidecar(SIDECAR_BINARY)?;
    let (mut rx, mut child) = sidecar.spawn()?;

    let bootstrap_line = build_bootstrap_line(handle, size)?;
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
    tauri::async_runtime::spawn(run_drain_loop(reader, app_handle));

    Ok(())
}

/// Drain loop: every [`DRAIN_INTERVAL`], drain the ring buffer, parse each
/// payload, emit the batch to the frontend.
async fn run_drain_loop<R: Runtime>(mut reader: RingBufReader, app: AppHandle<R>) {
    let mut ticker = tokio::time::interval(DRAIN_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        ticker.tick().await;
        let raw = reader.drain();
        if raw.is_empty() {
            continue;
        }

        let batch = parse_batch(&raw);
        if batch.is_empty() {
            continue;
        }

        if let Err(e) = app.emit("chat_messages", &batch) {
            tracing::error!(error = %e, "failed to emit chat_messages");
        }
    }
}
