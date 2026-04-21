// YouTube connection UI rendered inside the header.
//
// Three states drive what's shown:
// - logged_out:        button "Connect YouTube" → starts the PKCE flow
// - logged_out + busy: button shows "Waiting for browser…" with cancel
// - logged_in:         channel title + dropdown to Disconnect
//
// The flow itself: clicking the button opens the system browser at the
// Google authorization URL and immediately calls completeLogin() which
// blocks on the loopback redirect (see oauth_pkce::loopback). When the
// user authorizes in the browser, the loopback fires and the promise
// resolves. A `generation` counter guards against races between Cancel
// and a late-resolving completeLogin().

import { Component, Show, createSignal, onCleanup, onMount } from "solid-js";
import {
  cancelLogin,
  completeLogin,
  getAuthStatus,
  logout,
  openAuthorizationUri,
  startLogin,
  type AuthCommandError,
  type AuthStatus,
} from "../lib/youtubeAuth";

function isAuthError(e: unknown): e is AuthCommandError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as { kind?: unknown }).kind === "string"
  );
}

const YouTubeSignIn: Component = () => {
  const [status, setStatus] = createSignal<AuthStatus | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [menuOpen, setMenuOpen] = createSignal(false);
  let generation = 0;

  onMount(() => {
    void cancelLogin().catch(() => {});
    void getAuthStatus()
      .then(setStatus)
      .catch(() => setStatus({ state: "logged_out" }));
  });

  onCleanup(() => {
    generation += 1;
    void cancelLogin().catch(() => {});
  });

  const beginFlow = async () => {
    generation += 1;
    const gen = generation;
    setError(null);
    setBusy(true);
    try {
      const view = await startLogin();
      if (gen !== generation) return;
      void openAuthorizationUri(view.authorization_uri).catch(() => {});

      const next = await completeLogin();
      if (gen !== generation) return;
      setStatus(next);
    } catch (e) {
      if (gen !== generation) return;
      if (isAuthError(e)) {
        setError(authErrorMessage(e));
      } else {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (gen === generation) setBusy(false);
    }
  };

  const cancel = async () => {
    generation += 1;
    setBusy(false);
    setError(null);
    await cancelLogin().catch(() => {});
  };

  const disconnect = async () => {
    setMenuOpen(false);
    try {
      await logout();
      setStatus({ state: "logged_out" });
    } catch (e) {
      if (isAuthError(e)) setError(authErrorMessage(e));
      else setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div
      style={{
        position: "relative",
        display: "inline-flex",
        "align-items": "center",
        gap: "6px",
      }}
    >
      <YouTubeBadge connected={status()?.state === "logged_in"} />
      <Show
        when={(() => {
          const s = status();
          return s?.state === "logged_in" ? s : null;
        })()}
        fallback={
          <Show
            when={busy()}
            fallback={
              <button
                onClick={beginFlow}
                style={buttonStyle}
                title="Connect a YouTube account"
              >
                Connect YouTube
              </button>
            }
          >
            <span style={{ "font-size": "12px", color: "#aaa" }}>
              Waiting for browser…
            </span>
            <button onClick={cancel} style={subtleButton}>
              Cancel
            </button>
          </Show>
        }
      >
        {(loggedIn) => (
          <>
            <button
              onClick={() => setMenuOpen((o) => !o)}
              style={{
                ...subtleButton,
                "font-weight": 600,
                color: "#efeff1",
              }}
              title={`Connected as ${loggedIn().channel_title}`}
            >
              {loggedIn().channel_title}
            </button>
            <Show when={menuOpen()}>
              <div
                style={{
                  position: "absolute",
                  top: "calc(100% + 4px)",
                  right: 0,
                  background: "#222",
                  border: "1px solid #333",
                  "border-radius": "4px",
                  padding: "4px",
                  "min-width": "140px",
                  "z-index": 10,
                }}
              >
                <button
                  onClick={() => void disconnect()}
                  style={{
                    ...subtleButton,
                    width: "100%",
                    "text-align": "left",
                    color: "#ff6b6b",
                  }}
                >
                  Disconnect
                </button>
              </div>
            </Show>
          </>
        )}
      </Show>
      <Show when={error()}>
        {(e) => (
          <span
            style={{
              "font-size": "11px",
              color: "#ff6b6b",
              "max-width": "220px",
            }}
            title={e()}
          >
            {e()}
          </span>
        )}
      </Show>
    </div>
  );
};

const buttonStyle = {
  padding: "4px 10px",
  "font-size": "12px",
  background: "#cc0000",
  color: "white",
  border: "none",
  "border-radius": "4px",
  cursor: "pointer",
} as const;

const subtleButton = {
  padding: "4px 8px",
  "font-size": "12px",
  background: "transparent",
  color: "#888",
  border: "1px solid #333",
  "border-radius": "4px",
  cursor: "pointer",
} as const;

const YouTubeBadge: Component<{ connected: boolean }> = (props) => (
  <span
    title="YouTube"
    aria-label="YouTube"
    style={{
      display: "inline-flex",
      "align-items": "center",
      "justify-content": "center",
      width: "20px",
      height: "20px",
      "border-radius": "4px",
      background: props.connected ? "#cc0000" : "#3a3a3d",
    }}
  >
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      xmlns="http://www.w3.org/2000/svg"
    >
      <path
        d="M23.498 6.186a3.016 3.016 0 0 0-2.122-2.136C19.505 3.545 12 3.545 12 3.545s-7.505 0-9.377.505A3.017 3.017 0 0 0 .502 6.186C0 8.07 0 12 0 12s0 3.93.502 5.814a3.016 3.016 0 0 0 2.122 2.136c1.871.505 9.376.505 9.376.505s7.505 0 9.377-.505a3.015 3.015 0 0 0 2.122-2.136C24 15.93 24 12 24 12s0-3.93-.502-5.814zM9.545 15.568V8.432L15.818 12l-6.273 3.568z"
        fill="#fff"
      />
    </svg>
  </span>
);

function authErrorMessage(e: AuthCommandError): string {
  switch (e.kind) {
    case "user_denied":
      return "Authorization was denied. Try again to grant access.";
    case "loopback_bind":
      return `Could not start local listener: ${e.message}`;
    case "state_mismatch":
      return "Sign-in failed a security check. Try again.";
    case "no_channel":
      return "This Google account has no YouTube channel.";
    case "keychain":
      return `Could not access the OS credential store: ${e.message}`;
    case "oauth":
      return `Google rejected the request: ${e.message}`;
    case "no_pending_flow":
      return "Sign-in flow lost its state. Try again.";
    default:
      return e.message;
  }
}

export default YouTubeSignIn;
