mod host;
mod message;
pub mod ringbuf;
mod sidecar_supervisor;
pub mod twitch_auth;

pub mod emote_index;

// Re-exports for the bench harness. Gated so the public crate surface
// does not grow with bench-only plumbing in release builds.
#[cfg(any(test, feature = "__bench"))]
#[doc(hidden)]
pub use host::parse_batch;
#[cfg(any(test, feature = "__bench"))]
#[doc(hidden)]
pub use message::UnifiedMessage;

#[cfg(windows)]
use tauri::Manager;
use tauri::Runtime;
use tracing_subscriber::EnvFilter;

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

/// Tauri setup hook. On Windows, kicks off the sidecar supervisor which owns
/// the full lifecycle (spawn, bootstrap, drain, respawn-on-terminate). On
/// other platforms the supervisor is not wired up yet (ADR 18), so we log
/// and let the Tauri app launch without it so frontend work can proceed.
#[allow(clippy::unnecessary_wraps)]
fn setup<R: Runtime>(app: &mut tauri::App<R>) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        sidecar_supervisor::spawn(app.app_handle().clone());
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
