//! Developer tool: interactively seed the Twitch OAuth keychain entry.
//!
//! Runs the Device Code Grant flow once, waits for the user to authorize
//! in their browser, then persists the resulting access + refresh tokens
//! into the OS keychain (ADR 37). After this runs successfully, the
//! Tauri app supervisor's [`AuthManager::load_or_refresh`] picks up the
//! tokens and auto-refreshes them for the full 30-day refresh-token
//! lifetime without further manual steps.
//!
//! Usage:
//!
//! ```sh
//! export PRISMOID_TWITCH_CLIENT_ID=...
//! export PRISMOID_TWITCH_BROADCASTER_ID=...
//! cargo run --bin prismoid_dcf
//! ```
//!
//! The end-user DCF flow lands in a Tauri command + frontend button in
//! PRI-22; this is a dev-only tool.

use std::env;
use std::process::Command;

use prismoid_lib::twitch_auth::{AuthManager, KeychainStore};
use twitch_oauth2::Scope;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Dev-local .env.local fallback so this bin can be launched directly
    // from cargo without setting env vars every shell. Matches what
    // `lib::run` does for the Tauri entry point.
    #[cfg(debug_assertions)]
    let _ = dotenvy::from_filename(".env.local");

    let client_id =
        env::var("PRISMOID_TWITCH_CLIENT_ID").map_err(|_| "PRISMOID_TWITCH_CLIENT_ID not set")?;
    let broadcaster_id = env::var("PRISMOID_TWITCH_BROADCASTER_ID")
        .map_err(|_| "PRISMOID_TWITCH_BROADCASTER_ID not set")?;

    // OAuth HTTP client. Per oauth2-rs docs: disable redirects on the
    // client used for OAuth to avoid SSRF via redirect chains. Same
    // redirect-none pattern the supervisor uses.
    let http_client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;

    let mgr = AuthManager::builder(client_id)
        .scope(Scope::UserReadChat)
        .scope(Scope::UserWriteChat)
        .build(KeychainStore, http_client);

    println!("Starting Twitch Device Code Grant flow...");
    let pending = mgr.start_device_flow().await?;
    let details = pending.details();

    println!();
    println!("Open this URL in your browser and approve the authorization:");
    println!("  {}", details.verification_uri);
    println!();
    println!("Waiting for authorization (this polls until you click Authorize)...");

    // Best-effort: open the verification URL in the user's default browser.
    // Windows-specific; on macOS we'd use `open`, on Linux `xdg-open`. This
    // bin is a dev tool and ADR 36 locks Windows-first.
    let _ = Command::new("cmd")
        .args(["/c", "start", "", details.verification_uri.as_ref()])
        .spawn();

    let tokens = mgr.complete_device_flow(pending, &broadcaster_id).await?;

    println!();
    println!("✓ Authorized and persisted to keychain under `prismoid.twitch:{broadcaster_id}`");
    println!(
        "  access_token: [redacted, {} chars]",
        tokens.access_token.len()
    );
    println!(
        "  refresh_token: [redacted, {} chars]",
        tokens.refresh_token.len()
    );
    println!("  expires_at_ms: {}", tokens.expires_at_ms);
    println!("  scopes: {:?}", tokens.scopes);
    println!();
    println!("The supervisor will now auto-refresh this token within 5 min of expiry (ADR 29).");

    Ok(())
}
