// Pretext-powered measurement for chat messages. ADR 2: Pretext is the
// source of truth for text metrics so the virtual scroller gets exact
// pixel heights without DOM reflow.

import {
  measureRichInlineStats,
  prepareRichInline,
  type PreparedRichInline,
  type RichInlineItem,
} from "@chenglou/pretext/rich-inline";
import { measureNaturalWidth, prepareWithSegments } from "@chenglou/pretext";
import type { ChatMessage } from "../stores/chatStore";
import { splitMessage, type MessagePiece } from "./emoteSpans";

// Named families only. `system-ui` is unsafe for pretext accuracy on macOS.
// Exported so `ChatFeed.tsx` applies the exact same stack that Pretext
// measures against — any drift here produces wrong heights.
export const MESSAGE_FONT_FAMILY =
  '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif';
export const MESSAGE_FONT_SIZE_PX = 13;

const USERNAME_FONT = `700 ${MESSAGE_FONT_SIZE_PX}px ${MESSAGE_FONT_FAMILY}`;
const SEPARATOR_FONT = `400 ${MESSAGE_FONT_SIZE_PX}px ${MESSAGE_FONT_FAMILY}`;
const TEXT_FONT = `400 ${MESSAGE_FONT_SIZE_PX}px ${MESSAGE_FONT_FAMILY}`;

export const MESSAGE_LINE_HEIGHT = 20;
export const MESSAGE_PADDING_Y = 4;
// Horizontal padding applied to every message row; Pretext must measure
// against the container width minus both sides or the last-word-per-line
// overflow will never wrap and measured heights will be too small.
export const MESSAGE_PADDING_X = 8;

// NBSP is not collapsed by Pretext's boundary-whitespace rules, so it
// survives as a measurable placeholder. Each emote becomes an atomic
// (`break: "never"`) item whose total width equals the NBSP natural
// width plus `extraWidth`, sized to match the rendered <img>.
const EMOTE_PLACEHOLDER = "\u00A0";

const placeholderWidthCache = new Map<string, number>();
function placeholderWidth(font: string): number {
  const cached = placeholderWidthCache.get(font);
  if (cached !== undefined) return cached;
  const w = measureNaturalWidth(prepareWithSegments(EMOTE_PLACEHOLDER, font));
  placeholderWidthCache.set(font, w);
  return w;
}

export interface PreparedMessage {
  prepared: PreparedRichInline;
  pieces: MessagePiece[];
}

export function prepareMessage(msg: ChatMessage): PreparedMessage {
  const pieces = splitMessage(msg.message_text, msg.emote_spans, {
    maxHeight: MESSAGE_LINE_HEIGHT,
  });

  const items: RichInlineItem[] = [
    { text: msg.display_name, font: USERNAME_FONT, break: "never" },
    { text: ": ", font: SEPARATOR_FONT },
  ];

  const placeholder = placeholderWidth(TEXT_FONT);
  for (const piece of pieces) {
    if (piece.kind === "text") {
      items.push({ text: piece.text, font: TEXT_FONT });
    } else {
      items.push({
        text: EMOTE_PLACEHOLDER,
        font: TEXT_FONT,
        break: "never",
        extraWidth: Math.max(0, piece.primary.width - placeholder),
      });
    }
  }

  return { prepared: prepareRichInline(items), pieces };
}

export function measureMessageHeight(
  prepared: PreparedRichInline,
  containerWidth: number,
): number {
  const contentWidth = Math.max(0, containerWidth - MESSAGE_PADDING_X * 2);
  if (contentWidth <= 0) return MESSAGE_LINE_HEIGHT + MESSAGE_PADDING_Y;
  const { lineCount } = measureRichInlineStats(prepared, contentWidth);
  return Math.max(1, lineCount) * MESSAGE_LINE_HEIGHT + MESSAGE_PADDING_Y;
}
