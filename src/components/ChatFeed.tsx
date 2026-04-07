import { Component, For, createEffect, onMount, onCleanup } from "solid-js";
import { messages, addMessages, type ChatMessage } from "../stores/chatStore";
import { listen } from "@tauri-apps/api/event";

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

  createEffect(() => {
    messages();
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
      <For each={messages()}>
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
