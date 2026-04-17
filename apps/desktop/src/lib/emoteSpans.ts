// Splits a message into ordered text + emote pieces for both Pretext
// measurement and DOM rendering. EmoteSpan offsets from the Rust scanner
// are UTF-8 byte indices (see EmoteSpan docs in stores/chatStore.ts), so
// splicing with plain JS string operations is wrong for any non-ASCII
// text. We encode once and slice the byte array.

import type { EmoteMeta, EmoteSpan } from "../stores/chatStore";

export interface EmoteRenderInfo {
  emote: EmoteMeta;
  width: number;
  height: number;
}

export type MessagePiece =
  | { kind: "text"; text: string }
  | {
      kind: "emote";
      primary: EmoteRenderInfo;
      // Zero-width overlays stack on top of `primary` at the same x-origin.
      // They contribute no horizontal width to line layout.
      overlays: EmoteRenderInfo[];
    };

export interface SizeEmoteOptions {
  // Upper bound on rendered height. Emotes are scaled down proportionally
  // to this bound so chat line geometry stays predictable. Pass the
  // message line-height.
  maxHeight: number;
}

const FALLBACK_DIM = 28;

// Hoisted to module scope: TextEncoder/TextDecoder are stateless and the
// chat hot path calls splitMessage once per message, so reusing a single
// pair avoids per-call allocation pressure.
const utf8Encoder = new TextEncoder();
const utf8Decoder = new TextDecoder();

export function sizeEmote(
  emote: EmoteMeta,
  opts: SizeEmoteOptions,
): EmoteRenderInfo {
  const rawH = emote.height > 0 ? emote.height : FALLBACK_DIM;
  const rawW = emote.width > 0 ? emote.width : FALLBACK_DIM;
  const scale = rawH > opts.maxHeight ? opts.maxHeight / rawH : 1;
  return {
    emote,
    width: Math.max(1, Math.round(rawW * scale)),
    height: Math.max(1, Math.round(rawH * scale)),
  };
}

export function splitMessage(
  text: string,
  spans: EmoteSpan[],
  opts: SizeEmoteOptions,
): MessagePiece[] {
  if (spans.length === 0) {
    return text.length === 0 ? [] : [{ kind: "text", text }];
  }

  // The Rust scanner emits spans in ascending start order. Verify in a
  // single pass and only clone+sort on the rare violation; this keeps the
  // common path at O(n) with zero extra allocations.
  let sorted: EmoteSpan[] = spans;
  for (let i = 1; i < spans.length; i++) {
    if (spans[i]!.start < spans[i - 1]!.start) {
      sorted = spans.slice().sort((a, b) => a.start - b.start);
      break;
    }
  }

  const bytes = utf8Encoder.encode(text);

  const pieces: MessagePiece[] = [];
  let cursor = 0;

  for (const span of sorted) {
    if (
      span.start < cursor ||
      span.end < span.start ||
      span.end > bytes.length
    ) {
      // Malformed span; skip it rather than produce misaligned output.
      continue;
    }

    const sized = sizeEmote(span.emote, opts);

    if (span.emote.zero_width) {
      const prev = pieces.length > 0 ? pieces[pieces.length - 1] : undefined;
      if (prev && prev.kind === "emote") {
        // Flush any literal text between the previous emote and this one
        // before swallowing the zero-width span's code from the message.
        if (span.start > cursor) {
          pieces.push({
            kind: "text",
            text: utf8Decoder.decode(bytes.subarray(cursor, span.start)),
          });
        }
        prev.overlays.push(sized);
        cursor = span.end;
        continue;
      }
      // Orphan zero-width emote (no primary to stack on). Fall through and
      // render it as a normal inline emote.
    }

    if (span.start > cursor) {
      pieces.push({
        kind: "text",
        text: utf8Decoder.decode(bytes.subarray(cursor, span.start)),
      });
    }

    pieces.push({ kind: "emote", primary: sized, overlays: [] });
    cursor = span.end;
  }

  if (cursor < bytes.length) {
    pieces.push({
      kind: "text",
      text: utf8Decoder.decode(bytes.subarray(cursor)),
    });
  }

  return pieces;
}
