import { Component } from "solid-js";
import ChatFeed from "./components/ChatFeed";

const App: Component = () => {
  return (
    <div
      style={{ display: "flex", "flex-direction": "column", height: "100%" }}
    >
      <ChatFeed />
    </div>
  );
};

export default App;
