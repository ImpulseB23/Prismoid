package emotes

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"time"
)

// defaultHTTPClient is used when a provider is constructed without an explicit
// Doer. The 15s timeout covers 7TV/BTTV/FFZ p99 latencies from observed runs
// while still failing fast enough not to stall the channel-join flow.
var defaultHTTPClient = &http.Client{Timeout: 15 * time.Second}

// Doer is the subset of *http.Client the providers use. Tests swap in a
// transport-backed client.
type Doer interface {
	Do(req *http.Request) (*http.Response, error)
}

// ErrNotFound is returned when a channel has no set configured on the
// provider (e.g. 7TV 404 for users without a linked emote set). Callers
// treat it as "use global only", not an error to surface.
var ErrNotFound = errors.New("emotes: not found")

func getJSON(ctx context.Context, client Doer, url string, dst any) error {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return fmt.Errorf("build request: %w", err)
	}
	req.Header.Set("Accept", "application/json")

	resp, err := client.Do(req)
	if err != nil {
		return fmt.Errorf("request %s: %w", url, err)
	}
	defer func() { _ = resp.Body.Close() }()

	if resp.StatusCode == http.StatusNotFound {
		_, _ = io.Copy(io.Discard, resp.Body)
		return ErrNotFound
	}
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		body, _ := io.ReadAll(io.LimitReader(resp.Body, 512))
		return fmt.Errorf("GET %s: status %d: %s", url, resp.StatusCode, string(body))
	}

	if dst == nil {
		_, _ = io.Copy(io.Discard, resp.Body)
		return nil
	}
	if err := json.NewDecoder(resp.Body).Decode(dst); err != nil {
		return fmt.Errorf("decode %s: %w", url, err)
	}
	return nil
}
