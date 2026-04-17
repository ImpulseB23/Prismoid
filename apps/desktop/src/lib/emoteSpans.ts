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

  // Sort defensively; the scanner should already produce sorted spans but
  // a stray unsorted input here would corrupt every downstream slice.
  const sorted = [...spans].sort((a, b) => a.start - b.start);

  const encoder = new TextEncoder();
  const decoder = new TextDecoder();
  const bytes = encoder.encode(text);

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
            text: decoder.decode(bytes.subarray(cursor, span.start)),
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
        text: decoder.decode(bytes.subarray(cursor, span.start)),
      });
    }

    pieces.push({ kind: "emote", primary: sized, overlays: [] });
    cursor = span.end;
  }

  if (cursor < bytes.length) {
    pieces.push({
      kind: "text",
      text: decoder.decode(bytes.subarray(cursor)),
    });
  }

  return pieces;
}
