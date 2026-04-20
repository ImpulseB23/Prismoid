// Top chrome for the chat window. Shows the active channel, the platform
// it belongs to, and a live connection indicator fed by the supervisor's
// `sidecar_status` event. Purely presentational: no commands invoked, so
// a broken supervisor can't brick the UI.

import { Component, createSignal, onCleanup, onMount } from "solid-js";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  indicatorFor,
  type SidecarState,
  type SidecarStatus,
} from "../lib/sidecarStatus";

export interface HeaderProps {
  login: string;
}

const Header: Component<HeaderProps> = (props) => {
  const [state, setState] = createSignal<SidecarState | null>(null);
  let unlisten: UnlistenFn | undefined;
  // listen() is async, so a fast unmount can race the registration: if
  // onCleanup runs first, `unlisten` is still undefined and the listener
  // would leak when the promise resolves later. Track disposal explicitly
  // so the late-arriving handle can be torn down immediately.
  let disposed = false;

  onMount(() => {
    listen<SidecarStatus>("sidecar_status", (evt) => {
      setState(evt.payload.state);
    })
      .then((next) => {
        if (disposed) {
          next();
          return;
        }
        unlisten = next;
      })
      .catch((err: unknown) => {
        console.error("failed to subscribe to sidecar_status", err);
      });
  });

  onCleanup(() => {
    disposed = true;
    unlisten?.();
    unlisten = undefined;
  });

  return (
    <header
      style={{
        display: "flex",
        "align-items": "center",
        gap: "10px",
        padding: "8px 12px",
        "border-bottom": "1px solid #2a2a2e",
        background: "#18181b",
        "font-family":
          '-apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif',
        "font-size": "13px",
        color: "#efeff1",
        "flex-shrink": 0,
      }}
    >
      <span
        title="Twitch"
        aria-label="Twitch"
        style={{
          display: "inline-flex",
          "align-items": "center",
          "justify-content": "center",
          width: "20px",
          height: "20px",
          "border-radius": "4px",
          background: "#9146ff",
        }}
      >
        <svg
          width="12"
          height="12"
          viewBox="0 0 24 24"
          xmlns="http://www.w3.org/2000/svg"
        >
          <path
            d="M11.571 4.714h1.715v5.143H11.57zm4.715 0H18v5.143h-1.714zM6 0L1.714 4.286v15.428h5.143V24l4.286-4.286h3.428L22.286 12V0zm14.571 11.143l-3.428 3.428h-3.429l-3 3v-3H6.857V1.714h13.714Z"
            fill="#fff"
          />
        </svg>
      </span>
      <span style={{ "font-weight": 600 }}>{props.login}</span>
      <span style={{ flex: 1 }} />
      <StatusDot state={state()} />
    </header>
  );
};

const StatusDot: Component<{ state: SidecarState | null }> = (props) => {
  const info = () => indicatorFor(props.state);
  const tooltip = () => {
    const i = info();
    if (props.state === "running") return "Twitch: connected";
    return i.label;
  };
  return (
    <span
      data-testid="connection-dot"
      data-state={props.state ?? "initial"}
      title={tooltip()}
      style={{
        display: "inline-block",
        width: "8px",
        height: "8px",
        "border-radius": "50%",
        background: info().color,
        cursor: "default",
      }}
    />
  );
};

export default Header;
