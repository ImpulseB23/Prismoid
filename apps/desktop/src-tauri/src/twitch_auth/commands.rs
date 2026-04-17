//! Tauri command surface for the Twitch sign-in UI.
//!
//! These functions are thin adapters over [`AuthState`] (in `auth_state.rs`).
//! All branchable logic lives in `AuthState` and is unit-tested there;
//! these wrappers exist purely so `tauri::generate_handler!` can route
//! IPC calls. They're excluded from coverage in `codecov.yml` because
//! exercising them requires a Tauri runtime + WebView2 host process.
//!
//! Frontend flow:
//! 1. App boot → `twitch_auth_status` to render either the chat view
//!    (logged in) or the SignIn overlay (logged out).
//! 2. User clicks "Sign in with Twitch" → `twitch_start_login` returns
//!    the device-code details. The frontend renders the user_code,
//!    opens `verification_uri` in the system browser, and immediately
//!    calls `twitch_complete_login` which blocks until the user clicks
//!    Authorize (or the device code expires).
//! 3. On success, the supervisor's wakeup notifier fires so it picks
//!    up the new tokens without waiting out its 30 s `waiting_for_auth`
//!    sleep.
//! 4. `twitch_logout` wipes the keychain entry and re-shows the overlay
//!    on the next supervisor iteration.

use tauri::State;

use super::auth_state::{AuthCommandError, AuthState, AuthStatus, DeviceCodeView};

#[tauri::command]
pub async fn twitch_auth_status(
    state: State<'_, AuthState>,
) -> Result<AuthStatus, AuthCommandError> {
    state.status()
}

#[tauri::command]
pub async fn twitch_start_login(
    state: State<'_, AuthState>,
) -> Result<DeviceCodeView, AuthCommandError> {
    state.start_login().await
}

#[tauri::command]
pub async fn twitch_complete_login(
    state: State<'_, AuthState>,
) -> Result<AuthStatus, AuthCommandError> {
    state.complete_login().await
}

#[tauri::command]
pub async fn twitch_cancel_login(state: State<'_, AuthState>) -> Result<(), AuthCommandError> {
    state.cancel_login().await;
    Ok(())
}

#[tauri::command]
pub async fn twitch_logout(state: State<'_, AuthState>) -> Result<(), AuthCommandError> {
    state.logout().await
}
