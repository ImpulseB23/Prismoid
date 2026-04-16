// Virtualized chat renderer. Uses Pretext for exact pixel-perfect message
// heights (no DOM reflow), binary-searches the visible range, and keeps
// the mounted DOM bounded to the viewport window + overscan. ADR 21 +
// docs/frontend.md: one viewport signal per frame, message buffer lives
// outside Solid reactivity.

import {
  Component,
  For,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
} from "solid-js";
import { listen } from "@tauri-apps/api/event";
import type { PreparedRichInline } from "@chenglou/pretext/rich-inline";
import {
  addMessages,
  getMessage,
  viewport,
  type ChatMessage,
} from "../stores/chatStore";
import {
  MESSAGE_FONT_FAMILY,
  MESSAGE_FONT_SIZE_PX,
  MESSAGE_LINE_HEIGHT,
  MESSAGE_PADDING_X,
  MESSAGE_PADDING_Y,
  measureMessageHeight,
  prepareMessage,
} from "../lib/messageLayout";

const OVERSCAN = 6;
const STICK_THRESHOLD = 40;

interface PositionedMessage {
  monoIndex: number;
  msg: ChatMessage;
  prepared: PreparedRichInline;
  top: number;
  height: number;
}

const ChatFeed: Component = () => {
  let containerRef: HTMLDivElement | undefined;
  const preparedCache = new Map<number, PreparedRichInline>();

  const [width, setWidth] = createSignal(0);
  const [viewportHeight, setViewportHeight] = createSignal(0);
  const [scrollTop, setScrollTop] = createSignal(0);
  const [stickToBottom, setStickToBottom] = createSignal(true);
  const [fontsLoaded, setFontsLoaded] = createSignal(false);

  let scrollRafPending = false;

  const handleScroll = () => {
    if (!containerRef) return;
    if (scrollRafPending) return;
    scrollRafPending = true;
    requestAnimationFrame(() => {
      scrollRafPending = false;
      if (!containerRef) return;
      const top = containerRef.scrollTop;
      const clientH = containerRef.clientHeight;
      const totalH = containerRef.scrollHeight;
      setScrollTop(top);
      setStickToBottom(totalH - top - clientH <= STICK_THRESHOLD);
    });
  };

  const layout = createMemo<{
    messages: PositionedMessage[];
    totalHeight: number;
  }>(() => {
    const v = viewport();
    const w = width();
    if (!fontsLoaded() || w <= 0 || v.count === 0) {
      return { messages: [], totalHeight: 0 };
    }

    const liveStart = v.start;
    const liveEnd = v.start + v.count;

    for (const key of preparedCache.keys()) {
      if (key < liveStart) preparedCache.delete(key);
    }

    const messages: PositionedMessage[] = new Array(v.count);
    let y = 0;
    let writeIdx = 0;
    for (let mono = liveStart; mono < liveEnd; mono++) {
      const msg = getMessage(mono);
      if (!msg) continue;
      let prepared = preparedCache.get(mono);
      if (prepared === undefined) {
        prepared = prepareMessage(msg);
        preparedCache.set(mono, prepared);
      }
      const height = measureMessageHeight(prepared, w);
      messages[writeIdx++] = { monoIndex: mono, msg, prepared, top: y, height };
      y += height;
    }
    messages.length = writeIdx;
    return { messages, totalHeight: y };
  });

  const visibleRange = createMemo<{ start: number; end: number }>(() => {
    const { messages } = layout();
    const top = scrollTop();
    const vh = viewportHeight();
    if (messages.length === 0 || vh === 0) return { start: 0, end: 0 };

    const minY = Math.max(0, top);
    const maxY = top + vh;

    let low = 0;
    let high = messages.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (messages[mid]!.top + messages[mid]!.height > minY) high = mid;
      else low = mid + 1;
    }
    const start = Math.max(0, low - OVERSCAN);

    low = start;
    high = messages.length;
    while (low < high) {
      const mid = (low + high) >> 1;
      if (messages[mid]!.top >= maxY) high = mid;
      else low = mid + 1;
    }
    const end = Math.min(messages.length, low + OVERSCAN);
    return { start, end };
  });

  const visibleMessages = createMemo<PositionedMessage[]>(() => {
    const { messages } = layout();
    const { start, end } = visibleRange();
    return messages.slice(start, end);
  });

  createEffect(() => {
    const { totalHeight } = layout();
    if (!containerRef) return;
    if (!stickToBottom()) return;
    const vh = viewportHeight();
    if (vh === 0) return;
    const target = Math.max(0, totalHeight - vh);
    if (Math.abs(containerRef.scrollTop - target) > 0.5) {
      containerRef.scrollTop = target;
    }
  });

  onMount(() => {
    if (!containerRef) return;

    const ro = new ResizeObserver(() => {
      if (!containerRef) return;
      setWidth(containerRef.clientWidth);
      setViewportHeight(containerRef.clientHeight);
    });
    ro.observe(containerRef);
    setWidth(containerRef.clientWidth);
    setViewportHeight(containerRef.clientHeight);

    // Pretext uses canvas measureText; heights are only trustworthy once
    // webfonts are decoded. Fall back to immediate readiness in headless
    // environments that lack document.fonts, and on rejection/throw so the
    // UI never gets stuck waiting on a font promise that never settles.
    const fonts = (document as Document & { fonts?: FontFaceSet }).fonts;
    if (fonts && typeof fonts.ready?.then === "function") {
      fonts.ready
        .then(() => setFontsLoaded(true))
        .catch(() => setFontsLoaded(true));
    } else {
      setFontsLoaded(true);
    }

    let unlisten: (() => void) | undefined;
    listen<ChatMessage[]>("chat_messages", (event) => {
      addMessages(event.payload);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch((err) =>
        console.error("failed to listen for chat messages:", err),
      );

    onCleanup(() => {
      ro.disconnect();
      unlisten?.();
    });
  });

  return (
    <div
      ref={(el) => (containerRef = el)}
      onScroll={handleScroll}
      style={{
        flex: 1,
        "overflow-y": "auto",
        position: "relative",
        "will-change": "transform",
      }}
    >
      <div
        style={{
          position: "relative",
          height: `${layout().totalHeight}px`,
          width: "100%",
        }}
      >
        <For each={visibleMessages()}>
          {(item) => (
            <div
              style={{
                position: "absolute",
                top: 0,
                left: 0,
                right: 0,
                transform: `translateY(${item.top}px)`,
                height: `${item.height}px`,
                padding: `${MESSAGE_PADDING_Y / 2}px ${MESSAGE_PADDING_X}px`,
                "line-height": `${MESSAGE_LINE_HEIGHT}px`,
                "box-sizing": "border-box",
                "font-family": MESSAGE_FONT_FAMILY,
                "font-size": `${MESSAGE_FONT_SIZE_PX}px`,
                "white-space": "normal",
                "overflow-wrap": "break-word",
              }}
            >
              <span
                style={{
                  color: item.msg.color || "#9147ff",
                  "font-weight": 700,
                  // Keep DOM in lockstep with Pretext's `break: "never"`
                  // on the username segment so heights stay accurate even
                  // for very long display names.
                  "white-space": "nowrap",
                }}
              >
                {item.msg.display_name}
              </span>
              <span style={{ color: "#adadb8" }}>: </span>
              <span>{item.msg.message_text}</span>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default ChatFeed;
