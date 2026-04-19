import { afterEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/plugin-shell", () => ({
  open: vi.fn(),
}));

import { sendMessage, MAX_CHAT_MESSAGE_BYTES } from "./twitchAuth";

afterEach(() => {
  invokeMock.mockReset();
});

describe("sendMessage", () => {
  it("invokes twitch_send_message with the text payload", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await sendMessage("hello world");
    expect(invokeMock).toHaveBeenCalledWith("twitch_send_message", {
      text: "hello world",
    });
  });

  it("propagates the structured error from the backend", async () => {
    const err = { kind: "helix", code: "msg_duplicate", message: "dup" };
    invokeMock.mockRejectedValueOnce(err);
    await expect(sendMessage("x")).rejects.toEqual(err);
  });

  it("exposes the byte cap matching the Rust constant", () => {
    expect(MAX_CHAT_MESSAGE_BYTES).toBe(500);
  });
});
