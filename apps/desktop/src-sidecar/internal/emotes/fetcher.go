package emotes

import (
	"context"
	"errors"
	"sync"
)

// Bundle is the full set of emote and badge catalogs relevant to a single
// Twitch channel. Global entries are shared across channels; the host's
// emote index stores them once and joins per-channel sets on top.
//
// Any provider fetch that fails is reported in [Bundle.Errors] and the
// corresponding field is left zero-valued. A failing provider never fails
// the whole bundle — a dead BTTV CDN must not stop Twitch chat.
type Bundle struct {
	TwitchGlobalEmotes  EmoteSet
	TwitchChannelEmotes EmoteSet
	TwitchGlobalBadges  BadgeSet
	TwitchChannelBadges BadgeSet
	SevenTVGlobal       EmoteSet
	SevenTVChannel      EmoteSet
	BTTVGlobal          EmoteSet
	BTTVChannel         EmoteSet
	FFZGlobal           EmoteSet
	FFZChannel          EmoteSet
	Errors              []ProviderError
}

// ProviderError attributes a fetch failure to a specific provider and scope.
type ProviderError struct {
	Provider Provider
	Scope    Scope
	Err      error
}

func (e *ProviderError) Error() string {
	if e == nil {
		return "<nil>"
	}
	msg := string(e.Provider) + " " + string(e.Scope) + ": "
	if e.Err == nil {
		return msg + "<nil>"
	}
	return msg + e.Err.Error()
}

func (e *ProviderError) Unwrap() error {
	if e == nil {
		return nil
	}
	return e.Err
}

// Fetcher is the aggregate client for a single channel join. Each sub-client
// is optional: a nil client skips that provider entirely. The [TwitchClient]
// in particular may be nil when the user is unauthenticated; global and
// channel Twitch sets will simply be absent in that case.
type Fetcher struct {
	Twitch  *TwitchClient
	SevenTV *SevenTVClient
	BTTV    *BTTVClient
	FFZ     *FFZClient
}

// Fetch dispatches all enabled provider requests in parallel and returns a
// [Bundle]. [ErrNotFound] from a channel-scoped fetch is absorbed (treated
// as "no channel set configured") rather than recorded as an error.
func (f *Fetcher) Fetch(ctx context.Context, broadcasterID string) Bundle {
	var b Bundle
	var mu sync.Mutex
	var wg sync.WaitGroup

	record := func(p Provider, s Scope, err error) {
		if err == nil {
			return
		}
		// Channel-scoped 404s mean the broadcaster simply hasn't linked the
		// provider (no 7TV account, no FFZ room). A global 404 indicates an
		// outage or API change and must surface.
		if s == ScopeChannel && errors.Is(err, ErrNotFound) {
			return
		}
		mu.Lock()
		b.Errors = append(b.Errors, ProviderError{Provider: p, Scope: s, Err: err})
		mu.Unlock()
	}

	launch := func(fn func()) {
		wg.Add(1)
		go func() {
			defer wg.Done()
			fn()
		}()
	}

	if f.Twitch != nil {
		launch(func() {
			set, err := f.Twitch.FetchGlobalEmotes(ctx)
			record(ProviderTwitch, ScopeGlobal, err)
			if err == nil {
				mu.Lock()
				b.TwitchGlobalEmotes = set
				mu.Unlock()
			}
		})
		launch(func() {
			set, err := f.Twitch.FetchChannelEmotes(ctx, broadcasterID)
			record(ProviderTwitch, ScopeChannel, err)
			if err == nil {
				mu.Lock()
				b.TwitchChannelEmotes = set
				mu.Unlock()
			}
		})
		launch(func() {
			set, err := f.Twitch.FetchGlobalBadges(ctx)
			record(ProviderTwitch, ScopeGlobal, err)
			if err == nil {
				mu.Lock()
				b.TwitchGlobalBadges = set
				mu.Unlock()
			}
		})
		launch(func() {
			set, err := f.Twitch.FetchChannelBadges(ctx, broadcasterID)
			record(ProviderTwitch, ScopeChannel, err)
			if err == nil {
				mu.Lock()
				b.TwitchChannelBadges = set
				mu.Unlock()
			}
		})
	}

	if f.SevenTV != nil {
		launch(func() {
			set, err := f.SevenTV.FetchGlobal(ctx)
			record(Provider7TV, ScopeGlobal, err)
			if err == nil {
				mu.Lock()
				b.SevenTVGlobal = set
				mu.Unlock()
			}
		})
		launch(func() {
			set, err := f.SevenTV.FetchChannel(ctx, broadcasterID)
			record(Provider7TV, ScopeChannel, err)
			if err == nil {
				mu.Lock()
				b.SevenTVChannel = set
				mu.Unlock()
			}
		})
	}

	if f.BTTV != nil {
		launch(func() {
			set, err := f.BTTV.FetchGlobal(ctx)
			record(ProviderBTTV, ScopeGlobal, err)
			if err == nil {
				mu.Lock()
				b.BTTVGlobal = set
				mu.Unlock()
			}
		})
		launch(func() {
			set, err := f.BTTV.FetchChannel(ctx, broadcasterID)
			record(ProviderBTTV, ScopeChannel, err)
			if err == nil {
				mu.Lock()
				b.BTTVChannel = set
				mu.Unlock()
			}
		})
	}

	if f.FFZ != nil {
		launch(func() {
			set, err := f.FFZ.FetchGlobal(ctx)
			record(ProviderFFZ, ScopeGlobal, err)
			if err == nil {
				mu.Lock()
				b.FFZGlobal = set
				mu.Unlock()
			}
		})
		launch(func() {
			set, err := f.FFZ.FetchChannel(ctx, broadcasterID)
			record(ProviderFFZ, ScopeChannel, err)
			if err == nil {
				mu.Lock()
				b.FFZChannel = set
				mu.Unlock()
			}
		})
	}

	wg.Wait()
	return b
}
