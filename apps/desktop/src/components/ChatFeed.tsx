import {
  Component,
  For,
  createEffect,
  createMemo,
  onCleanup,
  onMount,
} from "solid-js";
import { listen } from "@tauri-apps/api/event";
import {
  addMessages,
  getMessage,
  viewport,
  type ChatMessage,
} from "../stores/chatStore";

const ChatFeed: Component = () => {
  let containerRef: HTMLDivElement | undefined;
  let userScrolledUp = false;

  const scrollToBottom = () => {
    if (!userScrolledUp && containerRef) {
      containerRef.scrollTop = containerRef.scrollHeight;
    }
  };

  const handleScroll = () => {
    if (!containerRef) return;
    const { scrollTop, scrollHeight, clientHeight } = containerRef;
    userScrolledUp = scrollHeight - scrollTop - clientHeight > 40;
  };

  // Derive the visible message slice from the viewport signal. The ring
  // buffer stores stable references, so `<For>` reuses DOM nodes for messages
  // that remain in the visible window across frames. The per-frame array
  // allocation is bounded by maxMessages and matches ADR 21's "one viewport
  // update per frame" contract.
  const visibleMessages = createMemo<ChatMessage[]>(() => {
    const v = viewport();
    const out: ChatMessage[] = [];
    for (let i = 0; i < v.count; i++) {
      const msg = getMessage(v.start + i);
      if (msg) out.push(msg);
    }
    return out;
  });

  createEffect(() => {
    visibleMessages();
    scrollToBottom();
  });

  onMount(() => {
    listen<ChatMessage[]>("chat_messages", (event) => {
      addMessages(event.payload);
    })
      .then((unlisten) => onCleanup(() => unlisten()))
      .catch((err) =>
        console.error("failed to listen for chat messages:", err),
      );
  });

  return (
    <div
      ref={(el) => (containerRef = el)}
      onScroll={handleScroll}
      style={{
        flex: 1,
        "overflow-y": "auto",
        padding: "8px",
        "will-change": "transform",
      }}
    >
      <For each={visibleMessages()}>
        {(msg) => (
          <div style={{ padding: "2px 0", "line-height": "1.4" }}>
            <span
              style={{ color: msg.color || "#9147ff", "font-weight": "bold" }}
            >
              {msg.display_name}
            </span>
            <span style={{ color: "#adadb8" }}>: </span>
            <span>{msg.message_text}</span>
          </div>
        )}
      </For>
    </div>
  );
};

export default ChatFeed;
