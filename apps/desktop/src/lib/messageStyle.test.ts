import { describe, expect, it } from "vitest";
import { formatTimestamp, normalizeUserColor } from "./messageStyle";

describe("normalizeUserColor", () => {
  it("returns the fallback for null/empty/invalid", () => {
    expect(normalizeUserColor(null)).toBe("#9147ff");
    expect(normalizeUserColor("")).toBe("#9147ff");
    expect(normalizeUserColor("not-a-color")).toBe("#9147ff");
    expect(normalizeUserColor("#xyz")).toBe("#9147ff");
  });

  it("passes through readable colors unchanged", () => {
    expect(normalizeUserColor("#ff7f50")).toBe("#ff7f50");
    expect(normalizeUserColor("#ffffff")).toBe("#ffffff");
  });

  it("expands 3-digit hex", () => {
    expect(normalizeUserColor("#fc8")).toBe("#ffcc88");
  });

  it("lifts pitch black to a visible grey", () => {
    const out = normalizeUserColor("#000000");
    expect(out).not.toBe("#000000");
    // Shouldn't blow out to white either; just enough to read.
    expect(out).not.toBe("#ffffff");
  });

  it("lifts deep blue to a readable blue", () => {
    const out = normalizeUserColor("#0000ff");
    // Channel zero gets lifted, so blue stays dominant but red/green grow.
    expect(out).toMatch(/^#[0-9a-f]{6}$/);
    expect(out).not.toBe("#0000ff");
  });
});

describe("formatTimestamp", () => {
  it("zero-pads hours and minutes", () => {
    // 2026-01-01T03:07:00 local time
    const d = new Date(2026, 0, 1, 3, 7, 0).getTime();
    expect(formatTimestamp(d)).toBe("03:07");
  });

  it("returns empty string for NaN", () => {
    expect(formatTimestamp(Number.NaN)).toBe("");
  });
});
