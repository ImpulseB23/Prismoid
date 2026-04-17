import { describe, expect, it } from "vitest";
import { createBadgeStore } from "./badgeStore";

function makeBadge(
  set: string,
  version: string,
  suffix = "",
): {
  set: string;
  version: string;
  title: string;
  url_1x: string;
  url_2x: string;
  url_4x: string;
} {
  return {
    set,
    version,
    title: `${set}/${version}${suffix}`,
    url_1x: `https://cdn/${set}/${version}/1x${suffix}.png`,
    url_2x: `https://cdn/${set}/${version}/2x${suffix}.png`,
    url_4x: `https://cdn/${set}/${version}/4x${suffix}.png`,
  };
}

describe("BadgeStore", () => {
  it("resolves a global badge", () => {
    const s = createBadgeStore();
    s.loadBundle({
      twitch_global_badges: { badges: [makeBadge("moderator", "1")] },
    });
    const r = s.resolve("moderator", "1");
    expect(r?.url_1x).toBe("https://cdn/moderator/1/1x.png");
  });

  it("channel overrides global", () => {
    const s = createBadgeStore();
    s.loadBundle({
      twitch_global_badges: {
        badges: [makeBadge("subscriber", "0", "-global")],
      },
      twitch_channel_badges: {
        badges: [makeBadge("subscriber", "0", "-channel")],
      },
    });
    const r = s.resolve("subscriber", "0");
    expect(r?.title).toBe("subscriber/0-channel");
  });

  it("returns undefined for unknown badge", () => {
    const s = createBadgeStore();
    s.loadBundle({ twitch_global_badges: { badges: [] } });
    expect(s.resolve("moderator", "1")).toBeUndefined();
  });

  it("bumps revision on every load", () => {
    const s = createBadgeStore();
    const r0 = s.revision();
    s.loadBundle({});
    s.loadBundle({});
    expect(s.revision()).toBe(r0 + 2);
  });

  it("later load replaces previous bundle", () => {
    const s = createBadgeStore();
    s.loadBundle({
      twitch_global_badges: { badges: [makeBadge("vip", "1")] },
    });
    s.loadBundle({
      twitch_global_badges: { badges: [makeBadge("moderator", "1")] },
    });
    expect(s.resolve("vip", "1")).toBeUndefined();
    expect(s.resolve("moderator", "1")).toBeDefined();
  });
});
