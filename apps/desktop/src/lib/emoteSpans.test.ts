import { describe, it, expect } from "vitest";
import { splitMessage, sizeEmote } from "./emoteSpans";
import type { EmoteMeta, EmoteSpan } from "../stores/chatStore";

function emote(overrides: Partial<EmoteMeta> = {}): EmoteMeta {
  return {
    id: "e1",
    code: "Kappa",
    provider: "twitch",
    url_1x: "https://example/1x",
    url_2x: "https://example/2x",
    url_4x: "https://example/4x",
    width: 28,
    height: 28,
    animated: false,
    zero_width: false,
    ...overrides,
  };
}

function span(
  start: number,
  end: number,
  meta: Partial<EmoteMeta> = {},
): EmoteSpan {
  return { start, end, emote: emote(meta) };
}

const OPTS = { maxHeight: 20 };

describe("splitMessage", () => {
  it("returns a single text piece when there are no spans", () => {
    expect(splitMessage("hello world", [], OPTS)).toEqual([
      { kind: "text", text: "hello world" },
    ]);
  });

  it("returns no pieces for empty input with no spans", () => {
    expect(splitMessage("", [], OPTS)).toEqual([]);
  });

  it("interleaves text and emotes in order", () => {
    // "hi Kappa bye": Kappa at bytes 3..8
    const pieces = splitMessage(
      "hi Kappa bye",
      [span(3, 8, { code: "Kappa" })],
      OPTS,
    );
    expect(pieces).toHaveLength(3);
    expect(pieces[0]).toEqual({ kind: "text", text: "hi " });
    expect(pieces[1]!.kind).toBe("emote");
    expect(pieces[2]).toEqual({ kind: "text", text: " bye" });
  });

  it("handles emote at start of message", () => {
    const pieces = splitMessage("Kappa hi", [span(0, 5)], OPTS);
    expect(pieces).toHaveLength(2);
    expect(pieces[0]!.kind).toBe("emote");
    expect(pieces[1]).toEqual({ kind: "text", text: " hi" });
  });

  it("handles emote at end of message", () => {
    const pieces = splitMessage("hi Kappa", [span(3, 8)], OPTS);
    expect(pieces).toHaveLength(2);
    expect(pieces[0]).toEqual({ kind: "text", text: "hi " });
    expect(pieces[1]!.kind).toBe("emote");
  });

  it("handles back-to-back emotes", () => {
    // "AB" where A and B are separate emote codes at bytes 0..1 and 1..2
    const pieces = splitMessage(
      "AB",
      [span(0, 1, { code: "A" }), span(1, 2, { code: "B" })],
      OPTS,
    );
    expect(pieces).toHaveLength(2);
    expect(pieces[0]!.kind).toBe("emote");
    expect(pieces[1]!.kind).toBe("emote");
  });

  it("uses UTF-8 byte offsets, not UTF-16 indices", () => {
    // "é" is 2 bytes in UTF-8 but 1 code unit in UTF-16. Message: "é Kappa"
    // Bytes: [0xC3, 0xA9, 0x20, 'K', 'a', 'p', 'p', 'a'] — Kappa at 3..8.
    const text = "é Kappa";
    const pieces = splitMessage(text, [span(3, 8)], OPTS);
    expect(pieces).toHaveLength(2);
    expect(pieces[0]).toEqual({ kind: "text", text: "é " });
    expect(pieces[1]!.kind).toBe("emote");
  });

  it("stacks zero-width overlays on the preceding emote", () => {
    const pieces = splitMessage(
      "A B",
      [span(0, 1, { code: "A" }), span(2, 3, { code: "B", zero_width: true })],
      OPTS,
    );
    expect(pieces).toHaveLength(2);
    expect(pieces[0]!.kind).toBe("emote");
    if (pieces[0]!.kind === "emote") {
      expect(pieces[0]!.overlays).toHaveLength(1);
      expect(pieces[0]!.overlays[0]!.emote.code).toBe("B");
    }
    expect(pieces[1]).toEqual({ kind: "text", text: " " });
  });

  it("renders orphan zero-width emote as inline when no prior emote exists", () => {
    const pieces = splitMessage(
      "hi X",
      [span(3, 4, { code: "X", zero_width: true })],
      OPTS,
    );
    expect(pieces).toHaveLength(2);
    expect(pieces[0]).toEqual({ kind: "text", text: "hi " });
    expect(pieces[1]!.kind).toBe("emote");
  });

  it("skips malformed spans without corrupting later output", () => {
    // Second span overlaps first; first is valid.
    const pieces = splitMessage(
      "hi Kappa bye",
      [span(3, 8), span(5, 10)],
      OPTS,
    );
    expect(pieces).toHaveLength(3);
    expect(pieces[0]).toEqual({ kind: "text", text: "hi " });
    expect(pieces[1]!.kind).toBe("emote");
    expect(pieces[2]).toEqual({ kind: "text", text: " bye" });
  });

  it("sorts out-of-order spans before splitting", () => {
    const pieces = splitMessage(
      "AB",
      [span(1, 2, { code: "B" }), span(0, 1, { code: "A" })],
      OPTS,
    );
    expect(pieces).toHaveLength(2);
    if (pieces[0]!.kind === "emote")
      expect(pieces[0]!.primary.emote.code).toBe("A");
    if (pieces[1]!.kind === "emote")
      expect(pieces[1]!.primary.emote.code).toBe("B");
  });
});

describe("sizeEmote", () => {
  it("scales down to fit maxHeight while preserving aspect ratio", () => {
    const sized = sizeEmote(emote({ width: 56, height: 56 }), {
      maxHeight: 20,
    });
    expect(sized.height).toBe(20);
    expect(sized.width).toBe(20);
  });

  it("leaves emotes smaller than maxHeight at native size", () => {
    const sized = sizeEmote(emote({ width: 18, height: 18 }), {
      maxHeight: 20,
    });
    expect(sized.height).toBe(18);
    expect(sized.width).toBe(18);
  });

  it("preserves non-square aspect ratio", () => {
    const sized = sizeEmote(emote({ width: 80, height: 40 }), {
      maxHeight: 20,
    });
    expect(sized.height).toBe(20);
    expect(sized.width).toBe(40);
  });

  it("falls back to 28x28 for zero-sized emotes", () => {
    const sized = sizeEmote(emote({ width: 0, height: 0 }), { maxHeight: 20 });
    expect(sized.width).toBeGreaterThan(0);
    expect(sized.height).toBeGreaterThan(0);
  });
});
