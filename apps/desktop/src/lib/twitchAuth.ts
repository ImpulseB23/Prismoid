// Twitch sign-in flow client. Mirrors the Tauri command surface in
// apps/desktop/src-tauri/src/twitch_auth/commands.rs.

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-shell";

export type AuthStatusState = "logged_out" | "logged_in";

// Discriminated union: when state is logged_in, login is guaranteed by
// the backend (twitch_auth::commands::twitch_auth_status). Modeling it
// this way prevents the UI from silently rendering an empty username if
// a malformed payload ever slips through.
export type AuthStatus =
  | { state: "logged_out" }
  | { state: "logged_in"; login: string };

export interface DeviceCodeView {
  verification_uri: string;
  user_code: string;
  expires_in_secs: number;
}

export interface AuthCommandError {
  kind:
    | "no_tokens"
    | "refresh_invalid"
    | "device_code_expired"
    | "user_denied"
    | "keychain"
    | "oauth"
    | "json"
    | "config"
    | "no_pending_flow";
  message: string;
}

export function getAuthStatus(): Promise<AuthStatus> {
  return invoke("twitch_auth_status");
}

export function startLogin(): Promise<DeviceCodeView> {
  return invoke("twitch_start_login");
}

export function completeLogin(): Promise<AuthStatus> {
  return invoke("twitch_complete_login");
}

export function cancelLogin(): Promise<void> {
  return invoke("twitch_cancel_login");
}

export function logout(): Promise<void> {
  return invoke("twitch_logout");
}

export function openVerificationUri(uri: string): Promise<void> {
  return open(uri);
}
