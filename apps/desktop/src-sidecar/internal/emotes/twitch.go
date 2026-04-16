package emotes

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"strings"
)

const twitchHelixDefaultBase = "https://api.twitch.tv/helix"

// TwitchClient fetches first-party emotes and chat badges from the Helix API.
//
// Kept intentionally separate from `internal/twitch.HelixClient`: the emotes
// package must not import the twitch package (the twitch package already
// pulls in EventSub/websocket dependencies, and these endpoints only need a
// plain Client-Id + Bearer pair).
type TwitchClient struct {
	HTTPClient  Doer
	BaseURL     string
	ClientID    string
	AccessToken string
}

// FetchGlobalEmotes returns Twitch's global emote set (Kappa, PogChamp,
// KEKW, …).
func (c *TwitchClient) FetchGlobalEmotes(ctx context.Context) (EmoteSet, error) {
	var raw twitchEmoteResponse
	if err := c.get(ctx, "/chat/emotes/global", &raw); err != nil {
		return EmoteSet{}, err
	}
	return EmoteSet{
		Provider: ProviderTwitch,
		Scope:    ScopeGlobal,
		Emotes:   twitchEmotesFromResponse(raw),
	}, nil
}

// FetchChannelEmotes returns the broadcaster's subscriber + bits + follower
// emote set. Empty (no error) when the channel has no custom emotes.
func (c *TwitchClient) FetchChannelEmotes(ctx context.Context, broadcasterID string) (EmoteSet, error) {
	q := url.Values{"broadcaster_id": []string{broadcasterID}}
	var raw twitchEmoteResponse
	if err := c.get(ctx, "/chat/emotes?"+q.Encode(), &raw); err != nil {
		return EmoteSet{}, err
	}
	return EmoteSet{
		Provider:  ProviderTwitch,
		Scope:     ScopeChannel,
		ChannelID: broadcasterID,
		Emotes:    twitchEmotesFromResponse(raw),
	}, nil
}

// FetchGlobalBadges returns the global chat badge set (subscriber base,
// moderator, verified, …). Per-channel subscriber-tier badges are returned
// from [FetchChannelBadges].
func (c *TwitchClient) FetchGlobalBadges(ctx context.Context) (BadgeSet, error) {
	var raw twitchBadgeResponse
	if err := c.get(ctx, "/chat/badges/global", &raw); err != nil {
		return BadgeSet{}, err
	}
	return BadgeSet{
		Scope:  ScopeGlobal,
		Badges: twitchBadgesFromResponse(raw),
	}, nil
}

// FetchChannelBadges returns per-channel badge overrides (custom subscriber
// tier art, bit tier art).
func (c *TwitchClient) FetchChannelBadges(ctx context.Context, broadcasterID string) (BadgeSet, error) {
	q := url.Values{"broadcaster_id": []string{broadcasterID}}
	var raw twitchBadgeResponse
	if err := c.get(ctx, "/chat/badges?"+q.Encode(), &raw); err != nil {
		return BadgeSet{}, err
	}
	return BadgeSet{
		Scope:     ScopeChannel,
		ChannelID: broadcasterID,
		Badges:    twitchBadgesFromResponse(raw),
	}, nil
}

func (c *TwitchClient) get(ctx context.Context, path string, dst any) error {
	base := c.BaseURL
	if base == "" {
		base = twitchHelixDefaultBase
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, base+path, nil)
	if err != nil {
		return fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Client-Id", c.ClientID)
	req.Header.Set("Authorization", "Bearer "+c.AccessToken)
	req.Header.Set("Accept", "application/json")

	client := c.HTTPClient
	if client == nil {
		client = defaultHTTPClient
	}
	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("request %s: %w", path, err)
	}
	defer func() { _ = resp.Body.Close() }()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return fmt.Errorf("GET %s: status %d: %s", path, resp.StatusCode, string(body))
	}
	if dst == nil {
		_, _ = io.Copy(io.Discard, resp.Body)
		return nil
	}
	if err := json.NewDecoder(resp.Body).Decode(dst); err != nil {
		return fmt.Errorf("decode %s: %w", path, err)
	}
	return nil
}

type twitchEmoteResponse struct {
	Data     []twitchEmote `json:"data"`
	Template string        `json:"template"`
}

type twitchEmote struct {
	ID        string   `json:"id"`
	Name      string   `json:"name"`
	Format    []string `json:"format"`
	Scale     []string `json:"scale"`
	ThemeMode []string `json:"theme_mode"`
}

type twitchBadgeResponse struct {
	Data []twitchBadgeSet `json:"data"`
}

type twitchBadgeSet struct {
	SetID    string           `json:"set_id"`
	Versions []twitchBadgeVer `json:"versions"`
}

type twitchBadgeVer struct {
	ID    string `json:"id"`
	URL1x string `json:"image_url_1x"`
	URL2x string `json:"image_url_2x"`
	URL4x string `json:"image_url_4x"`
	Title string `json:"title"`
}

func twitchEmotesFromResponse(r twitchEmoteResponse) []Emote {
	out := make([]Emote, 0, len(r.Data))
	for _, e := range r.Data {
		format := "static"
		animated := false
		for _, f := range e.Format {
			if f == "animated" {
				format = "animated"
				animated = true
				break
			}
		}
		mode := "dark"
		for _, m := range e.ThemeMode {
			if m == "dark" {
				mode = "dark"
				break
			}
			mode = m
		}
		em := Emote{
			ID:       e.ID,
			Code:     e.Name,
			Provider: ProviderTwitch,
			URL1x:    twitchEmoteURL(r.Template, e.ID, format, mode, "1.0"),
			URL2x:    twitchEmoteURL(r.Template, e.ID, format, mode, "2.0"),
			URL4x:    twitchEmoteURL(r.Template, e.ID, format, mode, "3.0"),
			Animated: animated,
		}
		if em.URL1x == "" {
			continue
		}
		out = append(out, em)
	}
	return out
}

// twitchEmoteURL expands the API's `template` field. Falls back to a hard-coded
// path if the response omits it (older cached responses sometimes do).
func twitchEmoteURL(template, id, format, theme, scale string) string {
	if template == "" {
		template = "https://static-cdn.jtvnw.net/emoticons/v2/{{id}}/{{format}}/{{theme_mode}}/{{scale}}"
	}
	r := strings.NewReplacer(
		"{{id}}", id,
		"{{format}}", format,
		"{{theme_mode}}", theme,
		"{{scale}}", scale,
	)
	return r.Replace(template)
}

func twitchBadgesFromResponse(r twitchBadgeResponse) []Badge {
	out := make([]Badge, 0, len(r.Data)*2)
	for _, set := range r.Data {
		for _, v := range set.Versions {
			if v.URL1x == "" {
				continue
			}
			out = append(out, Badge{
				Set:     set.SetID,
				Version: v.ID,
				Title:   v.Title,
				URL1x:   v.URL1x,
				URL2x:   v.URL2x,
				URL4x:   v.URL4x,
			})
		}
	}
	return out
}
