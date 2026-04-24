import { describe, expect, it } from "vitest";
import {
  fitsLimit,
  formatSendError,
  normalizeOutgoing,
  toSendError,
} from "./messageInput";

describe("normalizeOutgoing", () => {
  it("returns null for empty/whitespace", () => {
    expect(normalizeOutgoing("")).toBeNull();
    expect(normalizeOutgoing("   ")).toBeNull();
    expect(normalizeOutgoing("\n\t")).toBeNull();
  });

  it("trims surrounding whitespace", () => {
    expect(normalizeOutgoing("  hello  ")).toBe("hello");
  });
});

describe("fitsLimit", () => {
  it("accepts short ascii", () => {
    expect(fitsLimit("hello")).toBe(true);
  });

  it("rejects payloads larger than 500 bytes", () => {
    expect(fitsLimit("a".repeat(501))).toBe(false);
  });

  it("counts utf-8 bytes, not code units", () => {
    // Each emoji is 4 bytes in UTF-8; 126 of them = 504 bytes.
    expect(fitsLimit("🔥".repeat(126))).toBe(false);
    expect(fitsLimit("🔥".repeat(125))).toBe(true);
  });
});

describe("formatSendError", () => {
  it("formats each variant", () => {
    expect(formatSendError({ kind: "empty_message" })).toMatch(/empty/i);
    expect(
      formatSendError({ kind: "message_too_long", max_bytes: 500 }),
    ).toContain("500");
    expect(formatSendError({ kind: "sidecar_not_running" })).toMatch(/ready/i);
    expect(formatSendError({ kind: "not_logged_in", message: "x" })).toMatch(
      /sign in/i,
    );
    expect(formatSendError({ kind: "auth", message: "boom" })).toContain(
      "boom",
    );
    expect(formatSendError({ kind: "io", message: "pipe" })).toContain("pipe");
    expect(formatSendError({ kind: "json", message: "bad" })).toContain("bad");
    expect(
      formatSendError({
        kind: "helix",
        code: "msg_duplicate",
        message: "duplicate message",
      }),
    ).toContain("msg_duplicate");
    expect(
      formatSendError({ kind: "helix", code: "", message: "blocked" }),
    ).toContain("blocked");
    expect(
      formatSendError({
        kind: "message_too_long_chars",
        max_chars: 200,
      }),
    ).toContain("200");
    expect(
      formatSendError({
        kind: "youtube",
        code: "unauthorized",
        message: "x",
      }),
    ).toMatch(/sign in/i);
    expect(
      formatSendError({
        kind: "youtube",
        code: "quota_exceeded",
        message: "x",
      }),
    ).toMatch(/quota/i);
    expect(
      formatSendError({
        kind: "youtube",
        code: "youtube_api",
        message: "chat ended",
      }),
    ).toContain("chat ended");
    expect(
      formatSendError({ kind: "youtube", code: "", message: "blocked" }),
    ).toContain("blocked");
  });
});

describe("toSendError", () => {
  it("passes through valid structured errors", () => {
    const err = { kind: "empty_message" };
    expect(toSendError(err)).toBe(err);
    const helix = { kind: "helix", code: "x", message: "y" };
    expect(toSendError(helix)).toBe(helix);
  });

  it("accepts every known variant when shape matches", () => {
    const cases = [
      { kind: "empty_message" },
      { kind: "sidecar_not_running" },
      { kind: "not_logged_in", message: "x" },
      { kind: "io", message: "x" },
      { kind: "auth", message: "x" },
      { kind: "json", message: "x" },
      { kind: "message_too_long", max_bytes: 500 },
      { kind: "message_too_long_chars", max_chars: 200 },
      { kind: "helix", code: "c", message: "m" },
      { kind: "youtube", code: "c", message: "m" },
    ];
    for (const c of cases) {
      expect(toSendError(c)).toBe(c);
    }
  });

  it("rejects look-alike objects with the wrong shape", () => {
    // missing required `max_bytes`
    expect(toSendError({ kind: "message_too_long" })).toBe("[object Object]");
    // wrong type for required field
    expect(toSendError({ kind: "io", message: 5 })).toBe("[object Object]");
    // unknown variant
    expect(toSendError({ kind: "made_up_kind" })).toBe("[object Object]");
    // arrays should not be accepted
    expect(toSendError([{ kind: "empty_message" }])).not.toMatchObject({
      kind: "empty_message",
    });
  });

  it("stringifies unknown shapes", () => {
    expect(toSendError("nope")).toBe("nope");
    expect(toSendError(null)).toBe("null");
  });
});
