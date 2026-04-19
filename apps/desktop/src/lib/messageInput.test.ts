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
  });
});

describe("toSendError", () => {
  it("passes through structured errors", () => {
    const err = { kind: "empty_message" };
    expect(toSendError(err)).toBe(err);
  });

  it("stringifies unknown shapes", () => {
    expect(toSendError("nope")).toBe("nope");
    expect(toSendError(null)).toBe("null");
  });
});
