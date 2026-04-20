// Chat send input pinned below the message feed. Single-line: Enter
// sends, and the inline status row surfaces drop reasons or transport
// errors from the Tauri command.

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
import {
  buildOptimisticMessage,
  confirmPendingId,
  failPending,
  insertPending,
} from "../stores/chatStore";

interface Props {
  /** Current Twitch login of the signed-in user. Used as both the
   * username and provisional display name on the optimistic message;
   * the authoritative EventSub echo replaces both fields when it
   * arrives. */
  login: string;
}

const MessageInput: Component<Props> = (props) => {
  const [text, setText] = createSignal("");
  const [status, setStatus] = createSignal<string | null>(null);
  let inputEl: HTMLInputElement | undefined;
  // Per-input monotonic counter. Pending-entry mutations are already
  // safely keyed by `local_id`, but the inline status string is shared
  // across submits, so a slower earlier rejection could otherwise stomp
  // a newer success. We snapshot the seq at submit time and only let
  // the handler call `setStatus` when its snapshot is still the latest.
  let lastSubmitSeq = 0;

  const submit = () => {
    const payload = normalizeOutgoing(text());
    if (!payload) {
      setStatus("Message is empty.");
      return;
    }
    if (!fitsLimit(payload)) {
      setStatus(`Message exceeds ${MAX_CHAT_MESSAGE_BYTES} bytes.`);
      return;
    }
    setText("");
    setStatus(null);
    inputEl?.focus();

    const optimistic = buildOptimisticMessage({
      platform: "Twitch",
      login: props.login,
      text: payload,
    });
    insertPending(optimistic);
    const localId = optimistic.local_id!;
    const submitSeq = ++lastSubmitSeq;

    sendMessage(payload).then(
      (ok) => {
        confirmPendingId(localId, ok.message_id);
      },
      (raw) => {
        const err = toSendError(raw);
        const message =
          typeof err === "string"
            ? err
            : formatSendError(err as SendMessageError);
        failPending(localId, message);
        if (submitSeq === lastSubmitSeq) setStatus(message);
      },
    );
  };

  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter") {
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
          aria-label="Send a chat message"
          value={text()}
          placeholder="Send a message"
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
          disabled={normalizeOutgoing(text()) === null}
          onClick={() => void submit()}
          style={{
            "background-color": "#9147ff",
            color: "#fff",
            border: "none",
            "border-radius": "4px",
            padding: "6px 14px",
            "font-weight": 600,
            "font-size": "13px",
            cursor: "pointer",
          }}
        >
          Chat
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
