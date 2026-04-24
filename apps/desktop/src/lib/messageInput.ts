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
// suitable for an inline status line under the input.
export function formatSendError(err: SendMessageError): string {
  switch (err.kind) {
    case "not_logged_in":
      return "Sign in again to send messages.";
    case "empty_message":
      return "Message is empty.";
    case "message_too_long":
      return `Message exceeds ${err.max_bytes} bytes.`;
    case "message_too_long_chars":
      return `Message exceeds ${err.max_chars} characters.`;
    case "sidecar_not_running":
      return "Chat connection is not ready yet.";
    case "auth":
      return `Auth error: ${err.message}`;
    case "io":
      return `Connection error: ${err.message}`;
    case "json":
      return `Encoding error: ${err.message}`;
    case "helix":
      return err.code
        ? `Twitch rejected message (${err.code}): ${err.message}`
        : `Twitch rejected message: ${err.message}`;
    case "youtube":
      switch (err.code) {
        case "unauthorized":
          return "YouTube session expired. Sign in again.";
        case "quota_exceeded":
          return "YouTube daily quota exceeded. Try again later.";
        default:
          return err.code
            ? `YouTube rejected message (${err.code}): ${err.message}`
            : `YouTube rejected message: ${err.message}`;
      }
  }
}

// Per-variant required-field validators. Used by toSendError to make
// sure formatSendError never reads a field that doesn't exist on a
// look-alike object the backend didn't actually send.
const VARIANT_GUARDS: Record<
  SendMessageError["kind"],
  (v: Record<string, unknown>) => boolean
> = {
  empty_message: () => true,
  sidecar_not_running: () => true,
  not_logged_in: (v) => typeof v.message === "string",
  io: (v) => typeof v.message === "string",
  auth: (v) => typeof v.message === "string",
  json: (v) => typeof v.message === "string",
  message_too_long: (v) => typeof v.max_bytes === "number",
  message_too_long_chars: (v) => typeof v.max_chars === "number",
  helix: (v) => typeof v.code === "string" && typeof v.message === "string",
  youtube: (v) => typeof v.code === "string" && typeof v.message === "string",
};

// Strict guard for objects coming back from Tauri's invoke reject path.
// Only objects matching a known SendMessageError variant (correct
// `kind` and required field types) pass through as the structured
// shape; anything else is stringified so formatSendError is never
// handed a value missing the fields it expects.
export function toSendError(value: unknown): SendMessageError | string {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const obj = value as Record<string, unknown>;
    const kind = obj.kind;
    if (typeof kind === "string" && kind in VARIANT_GUARDS) {
      const guard = VARIANT_GUARDS[kind as SendMessageError["kind"]];
      if (guard(obj)) return obj as SendMessageError;
    }
  }
  return String(value);
}
