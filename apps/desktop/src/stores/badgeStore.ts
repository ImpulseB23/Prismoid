// Frontend-side badge lookup. Rust emits the full emote_bundle (which
// includes twitch_global_badges and twitch_channel_badges) once per
// channel join, and the badges on each ChatMessage arrive as opaque
// {set_id, id} pairs. This store resolves those pairs into rendered URLs
// using channel-overrides-global precedence, matching how the emote index
// resolves duplicate codes on the Rust side.
//
// Cross-platform plan: the {set_id, id} shape is platform-agnostic by
// design. Kick badges will pass through the same resolver (Kick's API
// embeds URLs per message, so the Kick parser will synthesize bundle
// entries directly). YouTube has no Helix-equivalent badge API, so
// semantic roles (owner/moderator/verified) will resolve to bundled SVG
// icons via a platform-scoped fallback layer; YT member badges already
// carry their own URL on the message and will land here as normal
// ResolvedBadge entries.

import { createSignal } from "solid-js";

export interface Badge {
  set: string;
  version: string;
  title: string;
  url_1x: string;
  url_2x: string;
  url_4x: string;
}

export interface BadgeSet {
  badges: Badge[];
}

export interface EmoteBundle {
  twitch_global_badges?: BadgeSet;
  twitch_channel_badges?: BadgeSet;
}

export interface ResolvedBadge {
  title: string;
  url_1x: string;
  url_2x: string;
  url_4x: string;
}

function keyFor(set: string, version: string): string {
  return `${set}\x1f${version}`;
}

export interface BadgeStore {
  loadBundle: (bundle: EmoteBundle) => void;
  resolve: (setId: string, id: string) => ResolvedBadge | undefined;
  /** Reactive revision that bumps on every bundle load. */
  revision: () => number;
}

export function createBadgeStore(): BadgeStore {
  let byKey = new Map<string, ResolvedBadge>();
  const [revision, setRevision] = createSignal(0);

  function ingest(
    target: Map<string, ResolvedBadge>,
    set: BadgeSet | undefined,
  ): void {
    if (!set) return;
    for (const b of set.badges) {
      target.set(keyFor(b.set, b.version), {
        title: b.title,
        url_1x: b.url_1x,
        url_2x: b.url_2x,
        url_4x: b.url_4x,
      });
    }
  }

  function loadBundle(bundle: EmoteBundle): void {
    const next = new Map<string, ResolvedBadge>();
    // Channel overrides global — mirrors EmoteIndex::load_bundle precedence.
    ingest(next, bundle.twitch_global_badges);
    ingest(next, bundle.twitch_channel_badges);
    byKey = next;
    setRevision((r) => r + 1);
  }

  function resolve(setId: string, id: string): ResolvedBadge | undefined {
    return byKey.get(keyFor(setId, id));
  }

  return { loadBundle, resolve, revision };
}

const defaultStore = createBadgeStore();
export const loadBadgeBundle = defaultStore.loadBundle;
export const resolveBadge = defaultStore.resolve;
export const badgeRevision = defaultStore.revision;
