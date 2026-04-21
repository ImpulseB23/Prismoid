//! Tauri command surface for the YouTube sign-in UI.
//!
//! Mirrors `twitch_auth::commands` shape — thin adapters over
//! [`AuthState`] which holds all the testable branchable logic.

use tauri::State;

use super::auth_state::{AuthCommandError, AuthState, AuthStatus, PkceFlowView};

#[tauri::command]
pub async fn youtube_auth_status(
    state: State<'_, AuthState>,
) -> Result<AuthStatus, AuthCommandError> {
    state.status()
}

#[tauri::command]
pub async fn youtube_start_login(
    state: State<'_, AuthState>,
) -> Result<PkceFlowView, AuthCommandError> {
    state.start_login().await
}

#[tauri::command]
pub async fn youtube_complete_login(
    state: State<'_, AuthState>,
) -> Result<AuthStatus, AuthCommandError> {
    state.complete_login().await
}

#[tauri::command]
pub async fn youtube_cancel_login(state: State<'_, AuthState>) -> Result<(), AuthCommandError> {
    state.cancel_login().await;
    Ok(())
}

#[tauri::command]
pub async fn youtube_logout(state: State<'_, AuthState>) -> Result<(), AuthCommandError> {
    state.logout().await
}
