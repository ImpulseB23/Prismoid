// Pure formatting + validation helpers for the chat send input. Kept
// out of the Solid component so they're testable in jsdom without
// pulling in @tauri-apps/api.

import { MAX_CHAT_MESSAGE_BYTES, type SendMessageError } from "./twitchAuth";

// Trim whitespace and reject blank input. Returns either the trimmed
// payload or null. The Tauri command also rejects blank input, but
// catching it locally keeps the UI snappy and avoids a needless RPC.
export function normalizeOutgoing(raw: string): string | null {
  const trimmed = raw.trim();
  if (trimmed.length === 0) return null;
  return trimmed;
}

// True if the encoded message fits inside Twitch's 500-byte cap.
// Counts UTF-8 bytes rather than JS string length so users typing in
// emoji, Cyrillic, etc. see the same limit Helix enforces.
export function fitsLimit(text: string): boolean {
  return new TextEncoder().encode(text).byteLength <= MAX_CHAT_MESSAGE_BYTES;
}

// Maps a structured Tauri command error into a short human message
// suitable for an inline status line under the input. Falls back to
// the raw `message` field for anything we don't have a tailored copy
// for, since the backend already includes a useful diagnostic.
export function formatSendError(err: SendMessageError): string {
  switch (err.kind) {
    case "not_logged_in":
      return "Sign in again to send messages.";
    case "empty_message":
      return "Message is empty.";
    case "message_too_long":
      return `Message exceeds ${err.max_bytes} bytes.`;
    case "sidecar_not_running":
      return "Chat connection is not ready yet.";
    case "auth":
      return `Auth error: ${err.message}`;
    case "io":
      return `Connection error: ${err.message}`;
    case "json":
      return `Encoding error: ${err.message}`;
  }
}

// Best-effort guard for objects coming back from Tauri's invoke reject
// path. The Rust side serializes the discriminated union with a `kind`
// tag, so anything carrying a string `kind` we treat as the structured
// shape; everything else gets stringified.
export function toSendError(value: unknown): SendMessageError | string {
  if (
    value &&
    typeof value === "object" &&
    "kind" in value &&
    typeof (value as { kind: unknown }).kind === "string"
  ) {
    return value as SendMessageError;
  }
  return String(value);
}
