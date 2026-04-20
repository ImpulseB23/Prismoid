// ADR 21 + docs/frontend.md: plain TS ring buffer outside Solid reactivity,
// one viewport signal per frame. The virtual scroller reads messages from
// the ring by monotonic index.

import { createSignal } from "solid-js";

export interface EmoteMeta {
  id: string;
  code: string;
  provider: "twitch" | "7tv" | "bttv" | "ffz";
  url_1x: string;
  url_2x: string;
  url_4x: string;
  width: number;
  height: number;
  animated: boolean;
  zero_width: boolean;
}

/**
 * One scanned emote occurrence inside `ChatMessage.message_text`.
 *
 * `start` and `end` are **UTF-8 byte offsets** as produced by the Rust
 * scanner, not UTF-16 code-unit offsets. JavaScript string indexing
 * (`String.prototype.slice`, `[]`, etc.) operates on UTF-16, so renderers
 * that splice the message around emote spans must translate first. The
 * straightforward way is to encode `message_text` once with `TextEncoder`
 * and slice the resulting `Uint8Array`, decoding each segment with
 * `TextDecoder`. For ASCII-only messages the two are equivalent.
 */
export interface EmoteSpan {
  start: number;
  end: number;
  emote: EmoteMeta;
}

export interface ChatMessage {
  id: string;
  platform: "Twitch" | "YouTube" | "Kick";
  timestamp: number;
  arrival_time: number;
  /**
   * Sort timestamp under the unified-ordering snap rule (see
   * `message.rs::compute_effective_ts`). Equals `timestamp` when the
   * platform clock agrees with local arrival within the snap window,
   * otherwise equals `arrival_time`. Use `(effective_ts, arrival_seq)`
   * as a stable sort key when interleaving messages from different
   * platforms or repositioning late arrivals.
   */
  effective_ts: number;
  /**
   * Per-process monotonic arrival counter assigned by the Rust drain
   * loop. Tie-breaks messages with identical `effective_ts` so two
   * renderers always agree on order.
   */
  arrival_seq: number;
  username: string;
  display_name: string;
  platform_user_id: string;
  message_text: string;
  badges: { set_id: string; id: string }[];
  is_mod: boolean;
  is_subscriber: boolean;
  is_broadcaster: boolean;
  color: string | null;
  reply_to: string | null;
  emote_spans: EmoteSpan[];
}

export interface Viewport {
  /** Monotonic index of the oldest message currently in the ring. */
  start: number;
  /** Number of valid messages in the ring (≤ maxMessages). */
  count: number;
}

export interface ChatStore {
  viewport: () => Viewport;
  addMessages: (batch: ChatMessage[]) => void;
  getMessage: (monoIndex: number) => ChatMessage | undefined;
}

export const DEFAULT_MAX_MESSAGES = 5000;

/**
 * Creates a chat store backed by a plain pre-allocated ring buffer. Writes
 * happen synchronously; the single viewport signal is batched into one
 * `requestAnimationFrame` tick so multiple batches arriving within the same
 * frame coalesce into exactly one reactive update.
 */
export function createChatStore(maxMessages = DEFAULT_MAX_MESSAGES): ChatStore {
  if (maxMessages <= 0) {
    throw new Error(`maxMessages must be positive, got ${maxMessages}`);
  }

  // Pre-allocated ring. Undefined slots only exist before writeIndex reaches
  // maxMessages for the first time; getMessage guards against reading them.
  const ring: (ChatMessage | undefined)[] = new Array<ChatMessage | undefined>(
    maxMessages,
  );
  let writeIndex = 0;
  let rafPending = false;

  const [viewport, setViewport] = createSignal<Viewport>({
    start: 0,
    count: 0,
  });

  function addMessages(batch: ChatMessage[]): void {
    if (batch.length === 0) return;
    for (const msg of batch) {
      ring[writeIndex % maxMessages] = msg;
      writeIndex++;
    }
    scheduleViewportUpdate();
  }

  function scheduleViewportUpdate(): void {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      setViewport({
        start: Math.max(0, writeIndex - maxMessages),
        count: Math.min(writeIndex, maxMessages),
      });
    });
  }

  function getMessage(monoIndex: number): ChatMessage | undefined {
    if (monoIndex < 0 || monoIndex >= writeIndex) return undefined;
    // evicted by wraparound
    if (monoIndex < writeIndex - maxMessages) return undefined;
    return ring[monoIndex % maxMessages];
  }

  return { viewport, addMessages, getMessage };
}

// Default singleton used by the production app.
const defaultStore = createChatStore();

export const viewport = defaultStore.viewport;
export const addMessages = defaultStore.addMessages;
export const getMessage = defaultStore.getMessage;
