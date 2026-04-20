import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { createChatStore, type ChatMessage } from "./chatStore";

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
