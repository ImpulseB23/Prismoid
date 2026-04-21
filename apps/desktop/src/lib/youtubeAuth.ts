// YouTube sign-in flow client. Mirrors the Tauri command surface in
// apps/desktop/src-tauri/src/youtube_auth/commands.rs.
//
// Flow shape differs from Twitch's DCF: there's no user_code to display
// — `start_login` returns an `authorization_uri` we open in the system
// browser, and `complete_login` blocks on the loopback HTTP redirect.
// The frontend's only job in between is to open the URL.

import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";

export type AuthStatusState = "logged_out" | "logged_in";

export type AuthStatus =
  | { state: "logged_out" }
  | { state: "logged_in"; channel_title: string };

export interface PkceFlowView {
  authorization_uri: string;
}

export interface AuthCommandError {
  kind:
    | "no_tokens"
    | "refresh_invalid"
    | "user_denied"
    | "loopback_bind"
    | "state_mismatch"
    | "keychain"
    | "oauth"
    | "json"
    | "no_channel"
    | "no_pending_flow";
  message: string;
}

export function getAuthStatus(): Promise<AuthStatus> {
  return invoke("youtube_auth_status");
}

export function startLogin(): Promise<PkceFlowView> {
  return invoke("youtube_start_login");
}

export function completeLogin(): Promise<AuthStatus> {
  return invoke("youtube_complete_login");
}

export function cancelLogin(): Promise<void> {
  return invoke("youtube_cancel_login");
}

export function logout(): Promise<void> {
  return invoke("youtube_logout");
}

const ALLOWED_HOSTS = ["accounts.google.com"];

export function openAuthorizationUri(uri: string): Promise<void> {
  let parsed: URL;
  try {
    parsed = new URL(uri);
  } catch {
    return Promise.reject(new Error("invalid authorization URL"));
  }
  if (
    parsed.protocol !== "https:" ||
    !ALLOWED_HOSTS.includes(parsed.hostname)
  ) {
    return Promise.reject(
      new Error("authorization URL not on a Google domain"),
    );
  }
  return openUrl(uri);
}
