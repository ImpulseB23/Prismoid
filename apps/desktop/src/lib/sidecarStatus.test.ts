import { describe, expect, it } from "vitest";
import { indicatorFor } from "./sidecarStatus";

describe("indicatorFor", () => {
  it("running → green connected", () => {
    expect(indicatorFor("running")).toEqual({
      label: "Connected",
      color: "#3fb950",
    });
  });

  it("spawning and backoff share the amber connecting indicator", () => {
    expect(indicatorFor("spawning")).toEqual({
      label: "Connecting",
      color: "#d29922",
    });
    expect(indicatorFor("backoff")).toEqual({
      label: "Connecting",
      color: "#d29922",
    });
  });

  it("waiting_for_auth surfaces a sign-in hint", () => {
    expect(indicatorFor("waiting_for_auth")).toEqual({
      label: "Waiting for sign-in",
      color: "#d29922",
    });
  });

  it("unhealthy and terminated are distinct", () => {
    expect(indicatorFor("unhealthy")).toEqual({
      label: "Unhealthy",
      color: "#db6d28",
    });
    expect(indicatorFor("terminated")).toEqual({
      label: "Disconnected",
      color: "#f85149",
    });
  });

  it("null before any event reports a starting state", () => {
    expect(indicatorFor(null)).toEqual({
      label: "Starting",
      color: "#6e7681",
    });
  });
});
