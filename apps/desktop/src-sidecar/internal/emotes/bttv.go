package emotes

import "context"

const bttvDefaultBase = "https://api.betterttv.net/3"
const bttvCDN = "https://cdn.betterttv.net"

// BTTVClient fetches global and channel emotes from BetterTTV.
//
// Docs: BTTV does not publish an OpenAPI spec; endpoints are stable and
// widely consumed by the community (Chatterino, FrankerFaceZ, Streamlink).
// Channel lookup 404s for any Twitch user without a BTTV account.
type BTTVClient struct {
	HTTPClient Doer
	BaseURL    string
}

// FetchGlobal returns BTTV's global emote set.
func (c *BTTVClient) FetchGlobal(ctx context.Context) (EmoteSet, error) {
	var raw []bttvEmote
	if err := getJSON(ctx, c.client(), c.base()+"/cached/emotes/global", &raw); err != nil {
		return EmoteSet{}, err
	}
	return EmoteSet{
		Provider: ProviderBTTV,
		Scope:    ScopeGlobal,
		Emotes:   bttvToEmotes(raw),
	}, nil
}

// FetchChannel returns BTTV's channel + shared emote sets for the given
// Twitch broadcaster. Returns [ErrNotFound] for channels without a BTTV
// integration; the channel and shared lists are merged.
func (c *BTTVClient) FetchChannel(ctx context.Context, twitchUserID string) (EmoteSet, error) {
	var raw bttvChannelResponse
	if err := getJSON(ctx, c.client(), c.base()+"/cached/users/twitch/"+twitchUserID, &raw); err != nil {
		return EmoteSet{}, err
	}
	merged := make([]bttvEmote, 0, len(raw.ChannelEmotes)+len(raw.SharedEmotes))
	merged = append(merged, raw.ChannelEmotes...)
	merged = append(merged, raw.SharedEmotes...)
	return EmoteSet{
		Provider:  ProviderBTTV,
		Scope:     ScopeChannel,
		ChannelID: twitchUserID,
		Emotes:    bttvToEmotes(merged),
	}, nil
}

func (c *BTTVClient) base() string {
	if c.BaseURL != "" {
		return c.BaseURL
	}
	return bttvDefaultBase
}

func (c *BTTVClient) client() Doer {
	if c.HTTPClient != nil {
		return c.HTTPClient
	}
	return defaultHTTPClient
}

type bttvEmote struct {
	ID        string `json:"id"`
	Code      string `json:"code"`
	ImageType string `json:"imageType"`
	Animated  bool   `json:"animated"`
	Width     int    `json:"width"`
	Height    int    `json:"height"`
}

type bttvChannelResponse struct {
	ChannelEmotes []bttvEmote `json:"channelEmotes"`
	SharedEmotes  []bttvEmote `json:"sharedEmotes"`
}

func bttvToEmotes(in []bttvEmote) []Emote {
	out := make([]Emote, 0, len(in))
	for _, e := range in {
		if e.ID == "" || e.Code == "" {
			continue
		}
		ext := e.ImageType
		if ext == "" {
			ext = "png"
		}
		// BTTV only derives animated from imageType=="gif" on legacy entries;
		// trust the explicit flag when present but infer from ext as a
		// fallback for older cached payloads that omit `animated`.
		animated := e.Animated || ext == "gif"
		out = append(out, Emote{
			ID:       e.ID,
			Code:     e.Code,
			Provider: ProviderBTTV,
			URL1x:    bttvURL(e.ID, "1x", ext),
			URL2x:    bttvURL(e.ID, "2x", ext),
			URL4x:    bttvURL(e.ID, "3x", ext),
			Width:    e.Width,
			Height:   e.Height,
			Animated: animated,
		})
	}
	return out
}

// bttvURL builds the CDN path for an emote at a given size variant.
// BTTV exposes 1x/2x/3x; we slot 3x into URL4x since it is the largest
// available.
func bttvURL(id, size, ext string) string {
	return bttvCDN + "/emote/" + id + "/" + size + "." + ext
}
