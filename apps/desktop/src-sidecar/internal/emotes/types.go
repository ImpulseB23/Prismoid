// Package emotes fetches third-party and first-party emote and badge
// catalogs (7TV, BTTV, FFZ, Twitch) and normalizes them into a common shape
// the Rust host can index with aho-corasick.
//
// Network I/O only. No parsing of chat messages, no caching — the host owns
// the index and the SQLite image cache (ADR 13).
package emotes

// Provider identifies which service an [Emote] or [Badge] originated from.
type Provider string

const (
	ProviderTwitch Provider = "twitch"
	Provider7TV    Provider = "7tv"
	ProviderBTTV   Provider = "bttv"
	ProviderFFZ    Provider = "ffz"
)

// Scope distinguishes global emote sets (available in every channel) from
// channel-specific sets (subscriber emotes, the channel's 7TV set, etc.).
type Scope string

const (
	ScopeGlobal  Scope = "global"
	ScopeChannel Scope = "channel"
)

// Emote is the normalized cross-provider shape fed into the Rust emote index.
// Width/Height are 0 when the provider does not report them; the renderer
// falls back to the first-frame decode size in that case.
type Emote struct {
	ID       string   `json:"id"`
	Code     string   `json:"code"`
	Provider Provider `json:"provider"`
	URL1x    string   `json:"url_1x"`
	URL2x    string   `json:"url_2x,omitempty"`
	URL4x    string   `json:"url_4x,omitempty"`
	Width    int      `json:"width,omitempty"`
	Height   int      `json:"height,omitempty"`
	Animated bool     `json:"animated,omitempty"`
	// ZeroWidth is the 7TV/BTTV "overlay" flag. True for emotes like
	// `cvMask`, `RainTime` that render on top of the previous emote.
	ZeroWidth bool `json:"zero_width,omitempty"`
}

// EmoteSet groups emotes by provider and scope. Channel sets carry the
// Twitch broadcaster ID they apply to.
type EmoteSet struct {
	Provider  Provider `json:"provider"`
	Scope     Scope    `json:"scope"`
	ChannelID string   `json:"channel_id,omitempty"`
	Emotes    []Emote  `json:"emotes"`
}

// Badge is a chat badge (subscriber, moderator, verified, etc.).
type Badge struct {
	// Set is the badge category: "subscriber", "moderator", "broadcaster"…
	Set string `json:"set"`
	// Version is the per-set variant: "0", "1", "12" for subscriber tiers,
	// "1" for most single-variant badges.
	Version string `json:"version"`
	Title   string `json:"title,omitempty"`
	URL1x   string `json:"url_1x"`
	URL2x   string `json:"url_2x,omitempty"`
	URL4x   string `json:"url_4x,omitempty"`
}

// BadgeSet groups badges by scope (global or per-channel).
type BadgeSet struct {
	Scope     Scope   `json:"scope"`
	ChannelID string  `json:"channel_id,omitempty"`
	Badges    []Badge `json:"badges"`
}
