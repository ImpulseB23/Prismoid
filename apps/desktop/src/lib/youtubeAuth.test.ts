import { afterEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));

vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: vi.fn().mockResolvedValue(undefined),
}));

import {
  cancelLogin,
  completeLogin,
  getAuthStatus,
  logout,
  openAuthorizationUri,
  startLogin,
} from "./youtubeAuth";

afterEach(() => {
  invokeMock.mockReset();
});

describe("youtubeAuth commands", () => {
  it("getAuthStatus invokes youtube_auth_status", async () => {
    invokeMock.mockResolvedValueOnce({ state: "logged_out" });
    await getAuthStatus();
    expect(invokeMock).toHaveBeenCalledWith("youtube_auth_status");
  });

  it("startLogin invokes youtube_start_login", async () => {
    invokeMock.mockResolvedValueOnce({ authorization_uri: "https://x" });
    await startLogin();
    expect(invokeMock).toHaveBeenCalledWith("youtube_start_login");
  });

  it("completeLogin invokes youtube_complete_login", async () => {
    invokeMock.mockResolvedValueOnce({
      state: "logged_in",
      channel_title: "T",
    });
    await completeLogin();
    expect(invokeMock).toHaveBeenCalledWith("youtube_complete_login");
  });

  it("cancelLogin invokes youtube_cancel_login", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await cancelLogin();
    expect(invokeMock).toHaveBeenCalledWith("youtube_cancel_login");
  });

  it("logout invokes youtube_logout", async () => {
    invokeMock.mockResolvedValueOnce(undefined);
    await logout();
    expect(invokeMock).toHaveBeenCalledWith("youtube_logout");
  });
});

describe("openAuthorizationUri", () => {
  it("allows accounts.google.com over https", async () => {
    await expect(
      openAuthorizationUri(
        "https://accounts.google.com/o/oauth2/v2/auth?client_id=x",
      ),
    ).resolves.toBeUndefined();
  });

  it("rejects non-Google hosts", async () => {
    await expect(
      openAuthorizationUri("https://evil.com/phish"),
    ).rejects.toThrow("authorization URL not on a Google domain");
  });

  it("rejects http URLs", async () => {
    await expect(
      openAuthorizationUri("http://accounts.google.com/o/oauth2/v2/auth"),
    ).rejects.toThrow("authorization URL not on a Google domain");
  });

  it("rejects invalid URLs", async () => {
    await expect(openAuthorizationUri("not-a-url")).rejects.toThrow(
      "invalid authorization URL",
    );
  });
});
