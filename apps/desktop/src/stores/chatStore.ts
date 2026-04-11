// ADR 21 + docs/frontend.md: plain TS ring buffer outside Solid reactivity,
// one viewport signal per frame. The virtual scroller reads messages from
// the ring by monotonic index.

import { createSignal } from "solid-js";

export interface ChatMessage {
  id: string;
  platform: "Twitch" | "YouTube" | "Kick";
  timestamp: number;
  arrival_time: number;
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
