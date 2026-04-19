// Chat send input. Single-line textarea-like input pinned below the
// message feed. Enter sends, Shift+Enter inserts a newline (Helix
// permits multi-line bodies), and the inline status row surfaces drop
// reasons or transport errors from the Tauri command.

import { Component, Show, createSignal } from "solid-js";
import {
  MAX_CHAT_MESSAGE_BYTES,
  sendMessage,
  type SendMessageError,
} from "../lib/twitchAuth";
import {
  fitsLimit,
  formatSendError,
  normalizeOutgoing,
  toSendError,
} from "../lib/messageInput";

const MessageInput: Component = () => {
  const [text, setText] = createSignal("");
  const [status, setStatus] = createSignal<string | null>(null);
  const [pending, setPending] = createSignal(false);
  let inputEl: HTMLInputElement | undefined;

  const submit = async () => {
    if (pending()) return;
    const payload = normalizeOutgoing(text());
    if (!payload) {
      setStatus("Message is empty.");
      return;
    }
    if (!fitsLimit(payload)) {
      setStatus(`Message exceeds ${MAX_CHAT_MESSAGE_BYTES} bytes.`);
      return;
    }
    setPending(true);
    setStatus(null);
    try {
      await sendMessage(payload);
      setText("");
      inputEl?.focus();
    } catch (raw) {
      const err = toSendError(raw);
      setStatus(
        typeof err === "string"
          ? err
          : formatSendError(err as SendMessageError),
      );
    } finally {
      setPending(false);
    }
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    }
  };

  return (
    <div
      style={{
        display: "flex",
        "flex-direction": "column",
        "border-top": "1px solid #2a2a2d",
        "background-color": "#1a1a1d",
      }}
    >
      <div
        style={{
          display: "flex",
          gap: "8px",
          padding: "8px",
          "align-items": "center",
        }}
      >
        <input
          ref={(el) => (inputEl = el)}
          type="text"
          value={text()}
          placeholder="Send a message"
          disabled={pending()}
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={onKeyDown}
          style={{
            flex: "1 1 auto",
            "background-color": "#0e0e10",
            color: "#efeff1",
            border: "1px solid #2a2a2d",
            "border-radius": "4px",
            padding: "6px 10px",
            "font-family":
              'ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
            "font-size": "13px",
            outline: "none",
          }}
        />
        <button
          type="button"
          disabled={pending() || normalizeOutgoing(text()) === null}
          onClick={() => void submit()}
          style={{
            "background-color": "#9147ff",
            color: "#fff",
            border: "none",
            "border-radius": "4px",
            padding: "6px 14px",
            "font-weight": 600,
            "font-size": "13px",
            cursor: pending() ? "default" : "pointer",
            opacity: pending() ? 0.6 : 1,
          }}
        >
          {pending() ? "Sending" : "Chat"}
        </button>
      </div>
      <Show when={status()}>
        <div
          style={{
            padding: "0 10px 6px",
            color: "#f5a3a3",
            "font-size": "12px",
            "font-family":
              'ui-sans-serif, system-ui, -apple-system, "Segoe UI", sans-serif',
          }}
        >
          {status()}
        </div>
      </Show>
    </div>
  );
};

export default MessageInput;
