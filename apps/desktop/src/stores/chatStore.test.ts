import { describe, it, expect } from "vitest";
import { messages, addMessages } from "./chatStore";

describe("chatStore", () => {
  it("adds messages to the buffer", () => {
    addMessages([
      {
        id: "1",
        platform: "Twitch",
        timestamp: Date.now(),
        arrival_time: Date.now(),
        username: "testuser",
        display_name: "TestUser",
        platform_user_id: "123",
        message_text: "hello world",
        badges: [],
        is_mod: false,
        is_subscriber: false,
        is_broadcaster: false,
        color: "#ff0000",
        reply_to: null,
      },
    ]);

    expect(messages().length).toBe(1);
    expect(messages()[0].message_text).toBe("hello world");
  });
});
