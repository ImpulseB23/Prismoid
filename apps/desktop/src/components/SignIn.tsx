import { Component, Show, createSignal, onMount } from "solid-js";
import {
  cancelLogin,
  completeLogin,
  openVerificationUri,
  startLogin,
  type AuthCommandError,
  type DeviceCodeView,
} from "../lib/twitchAuth";

export interface SignInProps {
  onAuthenticated: (login: string) => void;
}

function isAuthError(e: unknown): e is AuthCommandError {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as { kind?: unknown }).kind === "string"
  );
}

const SignIn: Component<SignInProps> = (props) => {
  const [pending, setPending] = createSignal<DeviceCodeView | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  // Tauri commands have no cancellation token, so an in-flight
  // completeLogin() Promise keeps polling Twitch even after the user
  // hits "Start over". The generation counter lets us ignore stale
  // resolutions/rejections from the previous attempt.
  let generation = 0;

  onMount(() => {
    void cancelLogin().catch(() => {});
  });

  const beginFlow = async () => {
    generation += 1;
    const gen = generation;
    setError(null);
    setBusy(true);
    try {
      const details = await startLogin();
      if (gen !== generation) return;
      setPending(details);
      void openVerificationUri(details.verification_uri).catch(() => {});

      const status = await completeLogin();
      if (gen !== generation) return;
      if (status.state === "logged_in" && status.login) {
        props.onAuthenticated(status.login);
      } else {
        setError("Sign-in returned without a login. Please try again.");
      }
    } catch (e) {
      if (gen !== generation) return;
      if (isAuthError(e)) {
        setError(authErrorMessage(e));
      } else {
        setError(e instanceof Error ? e.message : String(e));
      }
    } finally {
      if (gen === generation) {
        setBusy(false);
        setPending(null);
      }
    }
  };

  const startOver = async () => {
    generation += 1;
    setPending(null);
    setBusy(false);
    setError(null);
    await cancelLogin().catch(() => {});
  };

  return (
    <div
      style={{
        display: "flex",
        "flex-direction": "column",
        "align-items": "center",
        "justify-content": "center",
        height: "100%",
        gap: "16px",
        padding: "24px",
        "text-align": "center",
        "font-family": "system-ui, sans-serif",
      }}
    >
      <h1 style={{ margin: 0, "font-size": "20px" }}>Sign in to Twitch</h1>
      <p style={{ margin: 0, "max-width": "440px", color: "#888" }}>
        Prismoid needs your Twitch account to read chat and let you moderate and
        reply. We never see your password — Twitch handles authorization in your
        browser.
      </p>
      <Show
        when={pending()}
        fallback={
          <button
            onClick={beginFlow}
            disabled={busy()}
            style={{
              padding: "10px 20px",
              "font-size": "15px",
              "border-radius": "6px",
              border: "none",
              background: "#9146ff",
              color: "white",
              cursor: busy() ? "default" : "pointer",
            }}
          >
            Sign in with Twitch
          </button>
        }
      >
        {(p) => (
          <div
            style={{
              display: "flex",
              "flex-direction": "column",
              "align-items": "center",
              gap: "12px",
            }}
          >
            <p style={{ margin: 0 }}>
              A browser window opened. Confirm the code below, then click{" "}
              <strong>Authorize</strong>.
            </p>
            <code
              style={{
                "font-size": "24px",
                "letter-spacing": "4px",
                padding: "8px 16px",
                background: "#222",
                color: "white",
                "border-radius": "4px",
              }}
            >
              {p().user_code}
            </code>
            <p style={{ margin: 0, "font-size": "12px", color: "#888" }}>
              No browser?{" "}
              <a
                href="#"
                onClick={(e) => {
                  e.preventDefault();
                  void openVerificationUri(p().verification_uri);
                }}
                style={{ color: "#9146ff" }}
              >
                {p().verification_uri}
              </a>
            </p>
            <button
              onClick={startOver}
              style={{
                padding: "6px 12px",
                "font-size": "13px",
                background: "transparent",
                color: "#888",
                border: "1px solid #444",
                "border-radius": "4px",
                cursor: "pointer",
              }}
            >
              Start over
            </button>
          </div>
        )}
      </Show>
      <Show when={error()}>
        {(e) => (
          <p style={{ color: "#ff6b6b", margin: 0, "max-width": "440px" }}>
            {e()}
          </p>
        )}
      </Show>
    </div>
  );
};

function authErrorMessage(e: AuthCommandError): string {
  switch (e.kind) {
    case "user_denied":
      return "Authorization was denied. Try again to grant access.";
    case "device_code_expired":
      return "The code expired before you confirmed. Try again.";
    case "keychain":
      return `Could not access the OS credential store: ${e.message}`;
    case "oauth":
      return `Twitch rejected the request: ${e.message}`;
    case "no_pending_flow":
      return "Sign-in flow lost its state. Try again.";
    default:
      return e.message;
  }
}

export default SignIn;
