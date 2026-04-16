// Pretext-powered measurement for chat messages. ADR 2: Pretext is the
// source of truth for text metrics so the virtual scroller gets exact
// pixel heights without DOM reflow.

import {
  measureRichInlineStats,
  prepareRichInline,
  type PreparedRichInline,
} from "@chenglou/pretext/rich-inline";
import type { ChatMessage } from "../stores/chatStore";

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

export function prepareMessage(msg: ChatMessage): PreparedRichInline {
  return prepareRichInline([
    {
      text: msg.display_name,
      font: USERNAME_FONT,
      break: "never",
    },
    { text: ": ", font: SEPARATOR_FONT },
    { text: msg.message_text, font: TEXT_FONT },
  ]);
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
