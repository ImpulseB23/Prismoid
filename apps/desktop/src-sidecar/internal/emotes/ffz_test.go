package emotes

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestFFZ_FetchGlobal(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/set/global" {
			t.Fatalf("path: %s", r.URL.Path)
		}
		_, _ = w.Write([]byte(`{
			"default_sets":[3,4330],
			"sets":{
				"3":{"emoticons":[
					{"id":28138,"name":"LilZ","width":22,"height":30,
					 "urls":{"1":"//cdn.frankerfacez.com/emote/28138/1","2":"//cdn.frankerfacez.com/emote/28138/2","4":"//cdn.frankerfacez.com/emote/28138/4"}}
				]},
				"4330":{"emoticons":[
					{"id":9,"name":"ZrehplaR","width":16,"height":16,
					 "urls":{"1":"//cdn.frankerfacez.com/emote/9/1"},
					 "animated":{"1":"//cdn.frankerfacez.com/emote/9/animated/1"}}
				]},
				"99":{"emoticons":[{"id":1,"name":"Skip","urls":{}}]}
			}
		}`))
	}))
	defer srv.Close()

	c := &FFZClient{HTTPClient: srv.Client(), BaseURL: srv.URL}
	set, err := c.FetchGlobal(context.Background())
	if err != nil {
		t.Fatalf("FetchGlobal: %v", err)
	}
	// Set 99 is not in default_sets, so it must not appear.
	if len(set.Emotes) != 2 {
		t.Fatalf("emotes = %d, want 2", len(set.Emotes))
	}
	var lilz, zreh *Emote
	for i := range set.Emotes {
		switch set.Emotes[i].Code {
		case "LilZ":
			lilz = &set.Emotes[i]
		case "ZrehplaR":
			zreh = &set.Emotes[i]
		}
	}
	if lilz == nil || zreh == nil {
		t.Fatalf("missing emote: %+v", set.Emotes)
	}
	if lilz.URL1x != "https://cdn.frankerfacez.com/emote/28138/1" {
		t.Errorf("lilz URL1x: %q", lilz.URL1x)
	}
	if lilz.Animated {
		t.Error("lilz should not be animated")
	}
	if !zreh.Animated || zreh.URL1x != "https://cdn.frankerfacez.com/emote/9/animated/1" {
		t.Errorf("zreh: %+v", zreh)
	}
}

func TestFFZ_FetchChannel(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/room/id/77" {
			t.Fatalf("path: %s", r.URL.Path)
		}
		_, _ = w.Write([]byte(`{
			"room":{"set":500},
			"sets":{
				"500":{"emoticons":[{"id":1,"name":"ChanEmote","urls":{"1":"//cdn/x"}}]},
				"999":{"emoticons":[{"id":2,"name":"DraftEmote","urls":{"1":"//cdn/y"}}]}
			}
		}`))
	}))
	defer srv.Close()

	c := &FFZClient{HTTPClient: srv.Client(), BaseURL: srv.URL}
	set, err := c.FetchChannel(context.Background(), "77")
	if err != nil {
		t.Fatalf("FetchChannel: %v", err)
	}
	if set.ChannelID != "77" || len(set.Emotes) != 1 || set.Emotes[0].Code != "ChanEmote" {
		t.Errorf("unexpected (should only return the active set, not drafts): %+v", set)
	}
}

func TestFFZ_FetchChannel_NotFound(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusNotFound)
	}))
	defer srv.Close()

	c := &FFZClient{HTTPClient: srv.Client(), BaseURL: srv.URL}
	_, err := c.FetchChannel(context.Background(), "0")
	if !errors.Is(err, ErrNotFound) {
		t.Fatalf("err = %v, want ErrNotFound", err)
	}
}
