// Top chrome for the chat window. Shows the active channel, the platform
// it belongs to, and a live connection indicator fed by the supervisor's
// `sidecar_status` event. Purely presentational: no commands invoked, so
// a broken supervisor can't brick the UI.

import { Component, Show, createSignal, onCleanup, onMount } from "solid-js";
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
          color: "#fff",
          "font-weight": 700,
          "font-size": "12px",
        }}
      >
        T
      </span>
      <span style={{ "font-weight": 600 }}>{props.login}</span>
      <span style={{ flex: 1 }} />
      <ConnectionChip state={state()} />
    </header>
  );
};

const ConnectionChip: Component<{ state: SidecarState | null }> = (props) => {
  const info = () => indicatorFor(props.state);
  return (
    <span
      data-testid="connection-chip"
      data-state={props.state ?? "initial"}
      style={{
        display: "inline-flex",
        "align-items": "center",
        gap: "6px",
        padding: "2px 8px",
        "border-radius": "10px",
        background: "#1f1f23",
        border: "1px solid #2a2a2e",
        "font-size": "12px",
        color: "#c8c8d0",
      }}
    >
      <Dot color={info().color} />
      <Show when={info().label}>{info().label}</Show>
    </span>
  );
};

const Dot: Component<{ color: string }> = (props) => {
  return (
    <span
      style={{
        display: "inline-block",
        width: "8px",
        height: "8px",
        "border-radius": "50%",
        background: props.color,
      }}
    />
  );
};

export default Header;
