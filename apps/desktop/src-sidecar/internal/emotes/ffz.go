package emotes

import (
	"context"
	"strconv"
)

const ffzDefaultBase = "https://api.frankerfacez.com/v1"

// FFZClient fetches global and channel emotes from FrankerFaceZ.
//
// Docs: https://api.frankerfacez.com/docs/. Global and per-room responses
// both expose `sets[setID].emoticons[]`; the top-level `default_sets` /
// `room.set` tell us which sets are actually active for the lookup.
type FFZClient struct {
	HTTPClient Doer
	BaseURL    string
}

// FetchGlobal returns the merged default global FFZ sets.
func (c *FFZClient) FetchGlobal(ctx context.Context) (EmoteSet, error) {
	var raw ffzGlobalResponse
	if err := getJSON(ctx, c.client(), c.base()+"/set/global", &raw); err != nil {
		return EmoteSet{}, err
	}
	emotes := make([]Emote, 0, 128)
	for _, setID := range raw.DefaultSets {
		key := strconv.Itoa(setID)
		if set, ok := raw.Sets[key]; ok {
			emotes = append(emotes, ffzConvertSet(set)...)
		}
	}
	return EmoteSet{
		Provider: ProviderFFZ,
		Scope:    ScopeGlobal,
		Emotes:   emotes,
	}, nil
}

// FetchChannel returns the FFZ set configured for the Twitch broadcaster.
// Returns [ErrNotFound] when the channel has no FFZ room registered.
//
// FFZ's room response can include multiple sets in `sets` (historical data,
// unpublished drafts). Only the set identified by `room.set` is considered
// active by FFZ's own client and Chatterino, so that is all we convert.
func (c *FFZClient) FetchChannel(ctx context.Context, twitchUserID string) (EmoteSet, error) {
	var raw ffzRoomResponse
	if err := getJSON(ctx, c.client(), c.base()+"/room/id/"+twitchUserID, &raw); err != nil {
		return EmoteSet{}, err
	}
	emotes := make([]Emote, 0, 64)
	activeKey := strconv.Itoa(raw.Room.Set)
	if set, ok := raw.Sets[activeKey]; ok {
		emotes = append(emotes, ffzConvertSet(set)...)
	}
	return EmoteSet{
		Provider:  ProviderFFZ,
		Scope:     ScopeChannel,
		ChannelID: twitchUserID,
		Emotes:    emotes,
	}, nil
}

func (c *FFZClient) base() string {
	if c.BaseURL != "" {
		return c.BaseURL
	}
	return ffzDefaultBase
}

func (c *FFZClient) client() Doer {
	if c.HTTPClient != nil {
		return c.HTTPClient
	}
	return defaultHTTPClient
}

type ffzGlobalResponse struct {
	DefaultSets []int             `json:"default_sets"`
	Sets        map[string]ffzSet `json:"sets"`
}

type ffzRoomResponse struct {
	Room ffzRoom           `json:"room"`
	Sets map[string]ffzSet `json:"sets"`
}

// ffzRoom.Set is the ID of the active channel set. FFZ sometimes returns
// additional draft sets inside `sets` that the client should ignore.
type ffzRoom struct {
	Set int `json:"set"`
}

type ffzSet struct {
	Emoticons []ffzEmote `json:"emoticons"`
}

type ffzEmote struct {
	ID       int               `json:"id"`
	Name     string            `json:"name"`
	Width    int               `json:"width"`
	Height   int               `json:"height"`
	URLs     map[string]string `json:"urls"`
	Animated map[string]string `json:"animated"`
}

func ffzConvertSet(s ffzSet) []Emote {
	out := make([]Emote, 0, len(s.Emoticons))
	for _, e := range s.Emoticons {
		urls := e.URLs
		animated := false
		if len(e.Animated) > 0 {
			urls = e.Animated
			animated = true
		}
		u1 := normalizeFFZURL(urls["1"])
		if u1 == "" {
			continue
		}
		out = append(out, Emote{
			ID:       strconv.Itoa(e.ID),
			Code:     e.Name,
			Provider: ProviderFFZ,
			URL1x:    u1,
			URL2x:    normalizeFFZURL(urls["2"]),
			URL4x:    normalizeFFZURL(urls["4"]),
			Width:    e.Width,
			Height:   e.Height,
			Animated: animated,
		})
	}
	return out
}

// normalizeFFZURL upgrades FFZ's protocol-relative URLs to https. Empty
// input yields an empty string so callers can skip absent size variants.
func normalizeFFZURL(u string) string {
	if u == "" {
		return ""
	}
	if len(u) >= 2 && u[:2] == "//" {
		return "https:" + u
	}
	return u
}
