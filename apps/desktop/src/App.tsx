import { Component, Match, Switch, createSignal, onMount } from "solid-js";
import ChatFeed from "./components/ChatFeed";
import SignIn from "./components/SignIn";
import { getAuthStatus, type AuthStatusState } from "./lib/twitchAuth";

const App: Component = () => {
  // `null` = still loading initial status; the splash avoids a flash of
  // the SignIn overlay before the keychain check returns.
  const [authState, setAuthState] = createSignal<AuthStatusState | null>(null);

  onMount(async () => {
    try {
      const status = await getAuthStatus();
      setAuthState(status.state);
    } catch {
      // Treat any error from the status command as logged-out — the
      // SignIn flow surfaces a real error message if the underlying
      // keychain is broken.
      setAuthState("logged_out");
    }
  });

  return (
    <div
      style={{ display: "flex", "flex-direction": "column", height: "100%" }}
    >
      <Switch>
        <Match when={authState() === null}>
          <div
            style={{
              display: "flex",
              "align-items": "center",
              "justify-content": "center",
              height: "100%",
              color: "#888",
              "font-family": "system-ui, sans-serif",
            }}
          >
            Loading...
          </div>
        </Match>
        <Match when={authState() === "logged_out"}>
          <SignIn onAuthenticated={() => setAuthState("logged_in")} />
        </Match>
        <Match when={authState() === "logged_in"}>
          <ChatFeed />
        </Match>
      </Switch>
    </div>
  );
};

export default App;
