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
  /**
   * Optimistic-render state. `undefined` for confirmed messages (the
   * common case). `"pending"` while waiting for the platform echo,
   * `"failed"` when send was rejected. The renderer uses this to dim
   * pending entries and surface failed ones with a retry affordance.
   */
  status?: "pending" | "failed";
  /**
   * Client-generated id used to correlate an optimistic message with
   * its later platform echo. Only set on optimistic inserts.
   */
  local_id?: string;
  /** Human-readable failure reason; only set when status is "failed". */
  error_message?: string;
}

export interface Viewport {
  /** Monotonic index of the oldest message currently in the ring. */
  start: number;
  /** Number of valid messages in the ring (≤ maxMessages). */
  count: number;
}

/**
 * Window for matching an authoritative platform echo to a still-pending
 * optimistic message by fingerprint (platform + author + normalized
 * text). Generous enough to absorb sidecar cold-start latency without
 * letting genuine duplicates collapse onto stale pending entries.
 */
export const PENDING_FINGERPRINT_WINDOW_MS = 30_000;

/**
 * How many recent messages to scan when reconciling an incoming batch
 * against pending entries. Bounds the reconcile cost regardless of
 * pending-set size.
 */
export const PENDING_SCAN_TAIL = 64;

export interface ChatStore {
  viewport: () => Viewport;
  /**
   * Increments whenever an existing message in the ring is mutated in
   * place (pending reconcile, fail, or retry). Renderers that cache
   * per-message state (e.g. prepared layout) should subscribe and
   * invalidate when this changes; pure append paths bump the viewport
   * signal instead.
   */
  messageRevision: () => number;
  addMessages: (batch: ChatMessage[]) => void;
  getMessage: (monoIndex: number) => ChatMessage | undefined;
  /**
   * Inserts a locally-authored message in pending state. The caller
   * supplies a `local_id`; later calls to `confirmPendingId`,
   * `failPending`, and `retryPending` use it to find the entry. The
   * message renders immediately and is reconciled in place when its
   * authoritative platform echo arrives.
   */
  insertPending: (msg: ChatMessage) => void;
  /**
   * Records the Helix-assigned message id for a pending entry so a
   * subsequent echo can be matched by id (faster, more precise than
   * the fingerprint fallback). No-op if the entry has already been
   * reconciled or evicted.
   */
  confirmPendingId: (localId: string, messageId: string) => void;
  /**
   * Marks a pending entry as failed and stamps a human-readable
   * error. The message stays in the ring so the user can see what
   * went wrong and retry. No-op if already reconciled or evicted.
   */
  failPending: (localId: string, errorMessage: string) => void;
  /**
   * Resets a failed entry to pending state and returns its message
   * text so the caller can re-invoke the send command. Returns
   * `undefined` if the entry is missing or not in failed state.
   */
  retryPending: (localId: string) => string | undefined;
}

export const DEFAULT_MAX_MESSAGES = 5000;

/**
 * Frontend-only seq sentinel for optimistic pending messages. Set well
 * above any plausible per-session backend `arrival_seq` (which starts
 * at 0 and increments once per emitted message) so that when a future
 * sort layer compares `(effective_ts, arrival_seq)` across the ring,
 * pending entries naturally sort to the tail. Each pending insert
 * decrements the local counter from the same high base so multiple
 * concurrent pending messages still preserve submit order.
 */
const OPTIMISTIC_SEQ_BASE = Number.MAX_SAFE_INTEGER - 1_000_000;
let nextOptimisticSeq = OPTIMISTIC_SEQ_BASE;

export interface OptimisticInput {
  platform: ChatMessage["platform"];
  /** Login of the signed-in user; used as both username and provisional
   * display name on the optimistic entry. */
  login: string;
  /** Trimmed payload, exactly as it will be sent to the platform. */
  text: string;
}

/**
 * Builds a `ChatMessage` in pending state from the user's locally-known
 * identity. The `local_id` is generated here so the caller can hand it
 * straight to `confirmPendingId` / `failPending` after invoking the
 * send command. Stamps `effective_ts` with the local clock so the
 * snap rule keeps the message at "now" until reconciliation.
 */
export function buildOptimisticMessage(input: OptimisticInput): ChatMessage {
  const now = Date.now();
  const localId = generateLocalId();
  return {
    id: localId,
    platform: input.platform,
    timestamp: now,
    arrival_time: now,
    effective_ts: now,
    arrival_seq: nextOptimisticSeq++,
    username: input.login,
    display_name: input.login,
    platform_user_id: "",
    message_text: input.text,
    badges: [],
    is_mod: false,
    is_subscriber: false,
    is_broadcaster: false,
    color: null,
    reply_to: null,
    emote_spans: [],
    status: "pending",
    local_id: localId,
  };
}

function generateLocalId(): string {
  const c =
    typeof globalThis !== "undefined"
      ? (globalThis as { crypto?: Crypto }).crypto
      : undefined;
  if (c && typeof c.randomUUID === "function") {
    return c.randomUUID();
  }
  // Fallback for environments without WebCrypto. Not security-sensitive
  // here — only needs to be unique within the per-process pending set.
  return `local-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 10)}`;
}

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
  let pendingMutation = false;

  // localId → monoIndex of the pending entry. Stays in the map until
  // the entry is reconciled or evicted from the ring; we lazily prune
  // stale map entries when we discover an evicted monoIndex on access.
  const pendingByLocalId = new Map<string, number>();

  const [viewport, setViewport] = createSignal<Viewport>({
    start: 0,
    count: 0,
  });
  const [messageRevision, setMessageRevision] = createSignal(0);

  function isLive(monoIndex: number): boolean {
    return monoIndex >= 0 && monoIndex >= writeIndex - maxMessages;
  }

  function lookupPendingEntry(
    localId: string,
  ): { monoIndex: number; msg: ChatMessage } | undefined {
    const monoIndex = pendingByLocalId.get(localId);
    if (monoIndex === undefined) return undefined;
    if (!isLive(monoIndex)) {
      pendingByLocalId.delete(localId);
      return undefined;
    }
    const msg = ring[monoIndex % maxMessages];
    if (!msg || msg.local_id !== localId) {
      pendingByLocalId.delete(localId);
      return undefined;
    }
    return { monoIndex, msg };
  }

  function tryReconcile(incoming: ChatMessage): boolean {
    if (pendingByLocalId.size === 0) return false;
    const tailStart = Math.max(0, writeIndex - PENDING_SCAN_TAIL);
    for (let mono = writeIndex - 1; mono >= tailStart; mono--) {
      const entry = ring[mono % maxMessages];
      if (!entry || entry.status !== "pending") continue;
      if (entry.platform !== incoming.platform) continue;
      const idMatch = entry.id === incoming.id && entry.id.length > 0;
      const fingerprintMatch =
        entry.username === incoming.username &&
        entry.message_text === incoming.message_text &&
        Math.abs(incoming.arrival_time - entry.arrival_time) <
          PENDING_FINGERPRINT_WINDOW_MS;
      if (!idMatch && !fingerprintMatch) continue;
      reconcileEntry(entry, incoming);
      if (entry.local_id) pendingByLocalId.delete(entry.local_id);
      return true;
    }
    return false;
  }

  function reconcileEntry(entry: ChatMessage, incoming: ChatMessage): void {
    // Adopt authoritative identity and rendering data, but keep the
    // optimistic position (arrival_seq) so the row doesn't jump.
    entry.id = incoming.id;
    entry.timestamp = incoming.timestamp;
    entry.arrival_time = incoming.arrival_time;
    entry.effective_ts = incoming.effective_ts;
    entry.display_name = incoming.display_name;
    entry.platform_user_id = incoming.platform_user_id;
    entry.message_text = incoming.message_text;
    entry.badges = incoming.badges;
    entry.is_mod = incoming.is_mod;
    entry.is_subscriber = incoming.is_subscriber;
    entry.is_broadcaster = incoming.is_broadcaster;
    entry.color = incoming.color;
    entry.reply_to = incoming.reply_to;
    entry.emote_spans = incoming.emote_spans;
    entry.status = undefined;
    entry.error_message = undefined;
  }

  function addMessages(batch: ChatMessage[]): void {
    if (batch.length === 0) return;
    let appended = 0;
    let mutated = false;
    for (const msg of batch) {
      if (tryReconcile(msg)) {
        mutated = true;
        continue;
      }
      ring[writeIndex % maxMessages] = msg;
      writeIndex++;
      appended++;
    }
    if (appended > 0) scheduleViewportUpdate();
    if (mutated) bumpMessageRevision();
  }

  function insertPending(msg: ChatMessage): void {
    if (!msg.local_id) {
      throw new Error("insertPending requires msg.local_id");
    }
    msg.status = "pending";
    ring[writeIndex % maxMessages] = msg;
    pendingByLocalId.set(msg.local_id, writeIndex);
    writeIndex++;
    scheduleViewportUpdate();
  }

  function confirmPendingId(localId: string, messageId: string): void {
    const found = lookupPendingEntry(localId);
    if (!found || messageId.length === 0) return;
    found.msg.id = messageId;
    // No revision bump: id swap is invisible to renderers and the entry
    // is still pending. The reconcile-by-id path will pick it up when
    // the echo arrives.
  }

  function failPending(localId: string, errorMessage: string): void {
    const found = lookupPendingEntry(localId);
    if (!found) return;
    found.msg.status = "failed";
    found.msg.error_message = errorMessage;
    bumpMessageRevision();
  }

  function retryPending(localId: string): string | undefined {
    const found = lookupPendingEntry(localId);
    if (!found || found.msg.status !== "failed") return undefined;
    found.msg.status = "pending";
    found.msg.error_message = undefined;
    bumpMessageRevision();
    return found.msg.message_text;
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

  function bumpMessageRevision(): void {
    if (pendingMutation) return;
    pendingMutation = true;
    requestAnimationFrame(() => {
      pendingMutation = false;
      setMessageRevision((n) => n + 1);
    });
  }

  function getMessage(monoIndex: number): ChatMessage | undefined {
    if (monoIndex < 0 || monoIndex >= writeIndex) return undefined;
    // evicted by wraparound
    if (monoIndex < writeIndex - maxMessages) return undefined;
    return ring[monoIndex % maxMessages];
  }

  return {
    viewport,
    messageRevision,
    addMessages,
    getMessage,
    insertPending,
    confirmPendingId,
    failPending,
    retryPending,
  };
}

// Default singleton used by the production app.
const defaultStore = createChatStore();

export const viewport = defaultStore.viewport;
export const messageRevision = defaultStore.messageRevision;
export const addMessages = defaultStore.addMessages;
export const getMessage = defaultStore.getMessage;
export const insertPending = defaultStore.insertPending;
export const confirmPendingId = defaultStore.confirmPendingId;
export const failPending = defaultStore.failPending;
export const retryPending = defaultStore.retryPending;
