import { Component, Match, Switch, createSignal, onMount } from "solid-js";
import ChatFeed from "./components/ChatFeed";
import Header from "./components/Header";
import MessageInput from "./components/MessageInput";
import SignIn from "./components/SignIn";
import { getAuthStatus, type AuthStatus } from "./lib/twitchAuth";

const App: Component = () => {
  // `null` = still loading initial status; the splash avoids a flash of
  // the SignIn overlay before the keychain check returns.
  const [auth, setAuth] = createSignal<AuthStatus | null>(null);

  onMount(async () => {
    try {
      setAuth(await getAuthStatus());
    } catch {
      // Treat any error from the status command as logged-out — the
      // SignIn flow surfaces a real error message if the underlying
      // keychain is broken.
      setAuth({ state: "logged_out" });
    }
  });

  return (
    <div
      style={{ display: "flex", "flex-direction": "column", height: "100%" }}
    >
      <Switch>
        <Match when={auth() === null}>
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
        <Match when={auth()?.state === "logged_out"}>
          <SignIn
            onAuthenticated={(login) => setAuth({ state: "logged_in", login })}
          />
        </Match>
        <Match
          when={(() => {
            const a = auth();
            return a?.state === "logged_in" ? a : null;
          })()}
        >
          {(loggedIn) => (
            <>
              <Header login={loggedIn().login} />
              <ChatFeed />
              <MessageInput login={loggedIn().login} />
            </>
          )}
        </Match>
      </Switch>
    </div>
  );
};

export default App;
