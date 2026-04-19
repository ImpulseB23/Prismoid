// Pure state→indicator mapping shared with the header component.
// Split out so unit tests don't need to resolve the Solid/Tauri
// component module graph.

export type SidecarState =
  | "spawning"
  | "waiting_for_auth"
  | "backoff"
  | "running"
  | "unhealthy"
  | "terminated";

export interface SidecarStatus {
  state: SidecarState;
  attempt: number;
  backoff_ms?: number;
}

export interface Indicator {
  label: string;
  color: string;
}

export function indicatorFor(state: SidecarState | null): Indicator {
  switch (state) {
    case "running":
      return { label: "Connected", color: "#3fb950" };
    case "spawning":
    case "backoff":
      return { label: "Connecting", color: "#d29922" };
    case "waiting_for_auth":
      return { label: "Waiting for sign-in", color: "#d29922" };
    case "unhealthy":
      return { label: "Unhealthy", color: "#db6d28" };
    case "terminated":
      return { label: "Disconnected", color: "#f85149" };
    default:
      return { label: "Starting", color: "#6e7681" };
  }
}
