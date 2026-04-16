package emotes

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

const twitchGlobalEmotesBody = `{
  "data": [
    {"id":"25","name":"Kappa","format":["static"],"scale":["1.0","2.0","3.0"],"theme_mode":["light","dark"]},
    {"id":"88","name":"AnimTest","format":["static","animated"],"scale":["1.0","2.0","3.0"],"theme_mode":["dark"]}
  ],
  "template":"https://static-cdn.jtvnw.net/emoticons/v2/{{id}}/{{format}}/{{theme_mode}}/{{scale}}"
}`

func TestTwitch_FetchGlobalEmotes(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/chat/emotes/global" {
			t.Fatalf("path: %s", r.URL.Path)
		}
		if r.Header.Get("Client-Id") != "cid" || r.Header.Get("Authorization") != "Bearer tok" {
			t.Fatalf("missing auth headers: %v", r.Header)
		}
		_, _ = w.Write([]byte(twitchGlobalEmotesBody))
	}))
	defer srv.Close()

	c := &TwitchClient{HTTPClient: srv.Client(), BaseURL: srv.URL, ClientID: "cid", AccessToken: "tok"}
	set, err := c.FetchGlobalEmotes(context.Background())
	if err != nil {
		t.Fatalf("FetchGlobalEmotes: %v", err)
	}
	if len(set.Emotes) != 2 {
		t.Fatalf("emotes = %d, want 2", len(set.Emotes))
	}
	kappa := set.Emotes[0]
	if kappa.Code != "Kappa" {
		t.Fatalf("first = %q", kappa.Code)
	}
	if !strings.Contains(kappa.URL1x, "/25/static/dark/1.0") {
		t.Errorf("kappa URL1x = %q", kappa.URL1x)
	}
	anim := set.Emotes[1]
	if !anim.Animated || !strings.Contains(anim.URL1x, "/88/animated/dark/1.0") {
		t.Errorf("anim: %+v", anim)
	}
}

func TestTwitch_FetchChannelEmotes_BroadcasterQuery(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/chat/emotes" {
			t.Fatalf("path: %s", r.URL.Path)
		}
		if r.URL.Query().Get("broadcaster_id") != "123" {
			t.Fatalf("broadcaster_id: %q", r.URL.Query().Get("broadcaster_id"))
		}
		_, _ = w.Write([]byte(`{"data":[],"template":""}`))
	}))
	defer srv.Close()

	c := &TwitchClient{HTTPClient: srv.Client(), BaseURL: srv.URL, ClientID: "cid", AccessToken: "tok"}
	set, err := c.FetchChannelEmotes(context.Background(), "123")
	if err != nil {
		t.Fatalf("FetchChannelEmotes: %v", err)
	}
	if set.ChannelID != "123" || set.Scope != ScopeChannel {
		t.Errorf("bad set: %+v", set)
	}
}

func TestTwitch_FetchGlobalBadges(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/chat/badges/global" {
			t.Fatalf("path: %s", r.URL.Path)
		}
		_, _ = w.Write([]byte(`{"data":[
			{"set_id":"moderator","versions":[
				{"id":"1","image_url_1x":"https://u/1","image_url_2x":"https://u/2","image_url_4x":"https://u/4","title":"Moderator"}
			]},
			{"set_id":"broken","versions":[{"id":"1","image_url_1x":""}]}
		]}`))
	}))
	defer srv.Close()

	c := &TwitchClient{HTTPClient: srv.Client(), BaseURL: srv.URL, ClientID: "cid", AccessToken: "tok"}
	set, err := c.FetchGlobalBadges(context.Background())
	if err != nil {
		t.Fatalf("FetchGlobalBadges: %v", err)
	}
	if len(set.Badges) != 1 {
		t.Fatalf("badges = %d, want 1", len(set.Badges))
	}
	b := set.Badges[0]
	if b.Set != "moderator" || b.Version != "1" || b.URL4x != "https://u/4" {
		t.Errorf("badge: %+v", b)
	}
}

func TestTwitch_FetchChannelBadges(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Query().Get("broadcaster_id") != "555" {
			t.Fatalf("missing broadcaster_id")
		}
		_, _ = w.Write([]byte(`{"data":[{"set_id":"subscriber","versions":[{"id":"3000","image_url_1x":"https://s/1","title":"3-Year"}]}]}`))
	}))
	defer srv.Close()

	c := &TwitchClient{HTTPClient: srv.Client(), BaseURL: srv.URL, ClientID: "cid", AccessToken: "tok"}
	set, err := c.FetchChannelBadges(context.Background(), "555")
	if err != nil {
		t.Fatalf("FetchChannelBadges: %v", err)
	}
	if set.ChannelID != "555" || len(set.Badges) != 1 || set.Badges[0].Version != "3000" {
		t.Errorf("unexpected: %+v", set)
	}
}

func TestTwitch_ErrorStatusIsSurfaced(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusBadGateway)
		_, _ = w.Write([]byte(`{"error":"Bad Gateway","status":502}`))
	}))
	defer srv.Close()

	c := &TwitchClient{HTTPClient: srv.Client(), BaseURL: srv.URL, ClientID: "cid", AccessToken: "tok"}
	_, err := c.FetchGlobalEmotes(context.Background())
	if err == nil {
		t.Fatal("expected error")
	}
	if !strings.Contains(err.Error(), "502") {
		t.Errorf("err = %v", err)
	}
}
