package emotes

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"
)

const sevenTVSampleSet = `{
  "emotes": [
    {
      "id": "60ae958e229664e8667aea38",
      "name": "PepegaAim",
      "data": {
        "animated": true,
        "flags": 256,
        "host": {
          "url": "//cdn.7tv.app/emote/60ae958e229664e8667aea38",
          "files": [
            {"name": "1x.webp", "width": 32, "height": 32, "format": "WEBP"},
            {"name": "2x.webp", "width": 64, "height": 64, "format": "WEBP"},
            {"name": "4x.webp", "width": 128, "height": 128, "format": "WEBP"},
            {"name": "1x.avif", "width": 32, "height": 32, "format": "AVIF"}
          ]
        }
      }
    },
    {
      "id": "bad",
      "name": "Broken",
      "data": {"host": {"url": "//cdn.7tv.app/emote/bad", "files": []}}
    }
  ]
}`

func TestSevenTV_FetchGlobal(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/emote-sets/global" {
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(sevenTVSampleSet))
	}))
	defer srv.Close()

	c := &SevenTVClient{HTTPClient: srv.Client(), BaseURL: srv.URL}
	set, err := c.FetchGlobal(context.Background())
	if err != nil {
		t.Fatalf("FetchGlobal: %v", err)
	}
	if set.Provider != Provider7TV || set.Scope != ScopeGlobal {
		t.Errorf("wrong provider/scope: %+v", set)
	}
	if len(set.Emotes) != 1 {
		t.Fatalf("emotes = %d, want 1 (broken entry filtered)", len(set.Emotes))
	}
	e := set.Emotes[0]
	if e.Code != "PepegaAim" || !e.Animated || !e.ZeroWidth {
		t.Errorf("unexpected emote: %+v", e)
	}
	if e.URL1x != "https://cdn.7tv.app/emote/60ae958e229664e8667aea38/1x.webp" {
		t.Errorf("URL1x = %q", e.URL1x)
	}
	if e.URL4x == "" {
		t.Error("URL4x missing")
	}
	if e.Width != 32 || e.Height != 32 {
		t.Errorf("dims = %dx%d, want 32x32", e.Width, e.Height)
	}
}

func TestSevenTV_FetchChannel(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/users/twitch/12345" {
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
		_, _ = w.Write([]byte(`{"emote_set":` + sevenTVSampleSet + `}`))
	}))
	defer srv.Close()

	c := &SevenTVClient{HTTPClient: srv.Client(), BaseURL: srv.URL}
	set, err := c.FetchChannel(context.Background(), "12345")
	if err != nil {
		t.Fatalf("FetchChannel: %v", err)
	}
	if set.Scope != ScopeChannel || set.ChannelID != "12345" {
		t.Errorf("wrong scope/channel: %+v", set)
	}
	if len(set.Emotes) != 1 {
		t.Errorf("emotes = %d, want 1", len(set.Emotes))
	}
}

func TestSevenTV_FetchChannel_NotFound(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNotFound)
	}))
	defer srv.Close()

	c := &SevenTVClient{HTTPClient: srv.Client(), BaseURL: srv.URL}
	_, err := c.FetchChannel(context.Background(), "999")
	if !errors.Is(err, ErrNotFound) {
		t.Fatalf("err = %v, want ErrNotFound", err)
	}
}
