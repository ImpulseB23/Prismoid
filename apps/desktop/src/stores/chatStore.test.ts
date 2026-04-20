import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import {
  buildOptimisticMessage,
  createChatStore,
  type ChatMessage,
} from "./chatStore";

function makeMsg(id: string, text = `msg ${id}`): ChatMessage {
  return {
    id,
    platform: "Twitch",
    timestamp: 0,
    arrival_time: 0,
    effective_ts: 0,
    arrival_seq: 0,
    username: "u",
    display_name: "U",
    platform_user_id: "1",
    message_text: text,
    badges: [],
    is_mod: false,
    is_subscriber: false,
    is_broadcaster: false,
    color: null,
    reply_to: null,
    emote_spans: [],
  };
}

function makeAuthoritative(
  pending: ChatMessage,
  overrides: Partial<ChatMessage> = {},
): ChatMessage {
  return {
    ...makeMsg("from-platform", pending.message_text),
    platform: pending.platform,
    username: pending.username,
    arrival_time: pending.arrival_time + 200,
    timestamp: pending.arrival_time + 150,
    effective_ts: pending.arrival_time + 150,
    display_name: pending.username.toUpperCase(),
    platform_user_id: "real-id",
    badges: [{ set_id: "subscriber", id: "1" }],
    color: "#ff8800",
    ...overrides,
  };
}

describe("chatStore", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts with empty viewport", () => {
    const store = createChatStore(10);
    expect(store.viewport()).toEqual({ start: 0, count: 0 });
  });

  it("rejects non-positive maxMessages", () => {
    expect(() => createChatStore(0)).toThrow();
    expect(() => createChatStore(-5)).toThrow();
  });

  it("addMessages writes synchronously but defers viewport update to RAF", () => {
    const store = createChatStore(10);
    store.addMessages([makeMsg("1"), makeMsg("2")]);

    // Viewport is still stale before RAF fires.
    expect(store.viewport()).toEqual({ start: 0, count: 0 });
    // getMessage reflects writes immediately.
    expect(store.getMessage(0)?.id).toBe("1");
    expect(store.getMessage(1)?.id).toBe("2");

    vi.runAllTimers();

    expect(store.viewport()).toEqual({ start: 0, count: 2 });
  });

  it("coalesces multiple batches in the same frame into one viewport update", () => {
    const store = createChatStore(10);
    store.addMessages([makeMsg("1")]);
    store.addMessages([makeMsg("2")]);
    store.addMessages([makeMsg("3")]);

    expect(store.viewport().count).toBe(0);
    vi.runAllTimers();
    expect(store.viewport().count).toBe(3);

    // A second tick with nothing new should not change the viewport.
    vi.runAllTimers();
    expect(store.viewport().count).toBe(3);
  });

  it("empty batches are no-ops and do not schedule a frame", () => {
    const store = createChatStore(10);
    const rafSpy = vi.spyOn(globalThis, "requestAnimationFrame");
    store.addMessages([]);
    expect(rafSpy).not.toHaveBeenCalled();
    rafSpy.mockRestore();
  });

  it("wraps around and evicts the oldest messages", () => {
    const store = createChatStore(3);
    store.addMessages([makeMsg("1"), makeMsg("2"), makeMsg("3"), makeMsg("4")]);
    vi.runAllTimers();

    expect(store.viewport()).toEqual({ start: 1, count: 3 });
    // Evicted: index 0 should be undefined.
    expect(store.getMessage(0)).toBeUndefined();
    // Still present: 1, 2, 3.
    expect(store.getMessage(1)?.id).toBe("2");
    expect(store.getMessage(2)?.id).toBe("3");
    expect(store.getMessage(3)?.id).toBe("4");
  });

  it("getMessage returns undefined for out-of-range indices", () => {
    const store = createChatStore(5);
    store.addMessages([makeMsg("1"), makeMsg("2")]);
    vi.runAllTimers();

    expect(store.getMessage(-1)).toBeUndefined();
    expect(store.getMessage(2)).toBeUndefined();
    expect(store.getMessage(100)).toBeUndefined();
  });

  it("isolates state between stores", () => {
    const a = createChatStore(10);
    const b = createChatStore(10);
    a.addMessages([makeMsg("a1")]);
    b.addMessages([makeMsg("b1"), makeMsg("b2")]);
    vi.runAllTimers();

    expect(a.viewport().count).toBe(1);
    expect(b.viewport().count).toBe(2);
    expect(a.getMessage(0)?.id).toBe("a1");
    expect(b.getMessage(0)?.id).toBe("b1");
  });

  it("viewport.start advances monotonically with wraparound", () => {
    const store = createChatStore(3);
    store.addMessages([makeMsg("1"), makeMsg("2"), makeMsg("3")]);
    vi.runAllTimers();
    expect(store.viewport()).toEqual({ start: 0, count: 3 });

    store.addMessages([makeMsg("4")]);
    vi.runAllTimers();
    expect(store.viewport()).toEqual({ start: 1, count: 3 });

    store.addMessages([makeMsg("5"), makeMsg("6")]);
    vi.runAllTimers();
    expect(store.viewport()).toEqual({ start: 3, count: 3 });
  });

  it("only issues one requestAnimationFrame call per frame", () => {
    const store = createChatStore(10);
    const rafSpy = vi.spyOn(globalThis, "requestAnimationFrame");

    store.addMessages([makeMsg("1")]);
    store.addMessages([makeMsg("2")]);
    store.addMessages([makeMsg("3")]);
    expect(rafSpy).toHaveBeenCalledTimes(1);

    vi.runAllTimers();

    // After the flush, a new batch schedules a fresh frame.
    store.addMessages([makeMsg("4")]);
    expect(rafSpy).toHaveBeenCalledTimes(2);

    rafSpy.mockRestore();
  });
});

describe("chatStore optimistic send", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("buildOptimisticMessage marks status pending and assigns a local_id", () => {
    const msg = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "hi",
    });
    expect(msg.status).toBe("pending");
    expect(msg.local_id).toBeTruthy();
    expect(msg.id).toBe(msg.local_id);
    expect(msg.username).toBe("alice");
    expect(msg.display_name).toBe("alice");
    expect(msg.message_text).toBe("hi");
    expect(msg.effective_ts).toBe(msg.arrival_time);
  });

  it("insertPending appends to ring and bumps viewport", () => {
    const store = createChatStore(10);
    const pending = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "hi",
    });
    store.insertPending(pending);
    vi.runAllTimers();
    expect(store.viewport()).toEqual({ start: 0, count: 1 });
    expect(store.getMessage(0)?.status).toBe("pending");
  });

  it("addMessages reconciles incoming echo against pending by fingerprint", () => {
    const store = createChatStore(10);
    const pending = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "hi",
    });
    store.insertPending(pending);
    vi.runAllTimers();

    const auth = makeAuthoritative(pending);
    store.addMessages([auth]);
    vi.runAllTimers();

    // Echo did NOT append a second message; it merged into the pending entry.
    expect(store.viewport().count).toBe(1);
    const merged = store.getMessage(0)!;
    expect(merged.status).toBeUndefined();
    expect(merged.id).toBe("from-platform");
    expect(merged.display_name).toBe("ALICE");
    expect(merged.color).toBe("#ff8800");
    expect(merged.badges).toHaveLength(1);
  });

  it("addMessages reconciles by id once confirmPendingId is called", () => {
    const store = createChatStore(10);
    const pending = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "hello",
    });
    store.insertPending(pending);
    store.confirmPendingId(pending.local_id!, "helix-xyz");
    vi.runAllTimers();

    // Echo with matching id but DIFFERENT text still reconciles by id,
    // because the platform may normalize text in transit.
    const echo = makeAuthoritative(pending, {
      id: "helix-xyz",
      message_text: "hello",
    });
    store.addMessages([echo]);
    vi.runAllTimers();

    expect(store.viewport().count).toBe(1);
    expect(store.getMessage(0)?.status).toBeUndefined();
    expect(store.getMessage(0)?.id).toBe("helix-xyz");
  });

  it("does not reconcile when fingerprint window has elapsed", () => {
    const store = createChatStore(10);
    const pending = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "hi",
    });
    pending.arrival_time = 0;
    store.insertPending(pending);

    const stale = makeAuthoritative(pending);
    stale.arrival_time = 60_000; // far outside window
    store.addMessages([stale]);
    vi.runAllTimers();

    // Both kept: pending stays, stale is appended as a separate entry.
    expect(store.viewport().count).toBe(2);
    expect(store.getMessage(0)?.status).toBe("pending");
    expect(store.getMessage(1)?.status).toBeUndefined();
  });

  it("does not reconcile across platforms even with identical text", () => {
    const store = createChatStore(10);
    const pending = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "hi",
    });
    store.insertPending(pending);

    const echoOtherPlatform = makeAuthoritative(pending, {
      platform: "YouTube",
    });
    store.addMessages([echoOtherPlatform]);
    vi.runAllTimers();

    expect(store.viewport().count).toBe(2);
    expect(store.getMessage(0)?.status).toBe("pending");
  });

  it("failPending stamps error and bumps messageRevision", () => {
    const store = createChatStore(10);
    const pending = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "hi",
    });
    store.insertPending(pending);
    vi.runAllTimers();

    const revBefore = store.messageRevision();
    store.failPending(pending.local_id!, "rate limited");
    vi.runAllTimers();

    expect(store.getMessage(0)?.status).toBe("failed");
    expect(store.getMessage(0)?.error_message).toBe("rate limited");
    expect(store.messageRevision()).toBeGreaterThan(revBefore);
  });

  it("retryPending only flips a failed entry back to pending", () => {
    const store = createChatStore(10);
    const pending = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "retry me",
    });
    store.insertPending(pending);

    // No-op while still pending.
    expect(store.retryPending(pending.local_id!)).toBeUndefined();

    store.failPending(pending.local_id!, "boom");
    const text = store.retryPending(pending.local_id!);
    expect(text).toBe("retry me");
    expect(store.getMessage(0)?.status).toBe("pending");
    expect(store.getMessage(0)?.error_message).toBeUndefined();
  });

  it("confirmPendingId is a no-op when local_id is unknown", () => {
    const store = createChatStore(10);
    // Should not throw.
    store.confirmPendingId("never-existed", "helix-1");
  });

  it("scan is bounded and ignores pending entries past PENDING_SCAN_TAIL", () => {
    const store = createChatStore(200);
    const oldPending = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "old one",
    });
    store.insertPending(oldPending);

    // Push 100 unrelated messages so the pending falls outside the
    // 64-entry reconciliation tail.
    const filler: ChatMessage[] = [];
    for (let i = 0; i < 100; i++) filler.push(makeMsg(`f${i}`));
    store.addMessages(filler);

    const echo = makeAuthoritative(oldPending);
    store.addMessages([echo]);
    vi.runAllTimers();

    // Echo could not find the pending entry: it appends as a normal msg
    // and the original pending stays in pending state.
    expect(store.getMessage(0)?.status).toBe("pending");
  });

  it("multiple pending messages reconcile in submit order via fingerprint", () => {
    const store = createChatStore(20);
    const a = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "first",
    });
    const b = buildOptimisticMessage({
      platform: "Twitch",
      login: "alice",
      text: "second",
    });
    store.insertPending(a);
    store.insertPending(b);
    vi.runAllTimers();

    store.addMessages([makeAuthoritative(a, { id: "id-a" })]);
    store.addMessages([makeAuthoritative(b, { id: "id-b" })]);
    vi.runAllTimers();

    expect(store.viewport().count).toBe(2);
    expect(store.getMessage(0)?.id).toBe("id-a");
    expect(store.getMessage(1)?.id).toBe("id-b");
    expect(store.getMessage(0)?.status).toBeUndefined();
    expect(store.getMessage(1)?.status).toBeUndefined();
  });
});
