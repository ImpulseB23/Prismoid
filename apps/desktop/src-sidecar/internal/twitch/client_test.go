package twitch

import (
	"context"
	"errors"
	"net/http"
	"net/http/httptest"
	"strconv"
	"sync/atomic"
	"testing"
	"time"
)

type followResp struct {
	Data []struct {
		UserName string `json:"user_name"`
	} `json:"data"`
}

func newHelixTestClient(srv *httptest.Server) *HelixClient {
	return &HelixClient{
		HTTPClient:  srv.Client(),
		BaseURL:     srv.URL,
		ClientID:    "cid",
		AccessToken: "tok",
	}
}

func TestHelixClient_GetSendsAuthHeadersAndDecodesBody(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet {
			t.Fatalf("expected GET, got %s", r.Method)
		}
		if r.URL.Path != "/users/follows" {
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
		if got := r.Header.Get("Client-Id"); got != "cid" {
			t.Errorf("Client-Id = %q, want cid", got)
		}
		if got := r.Header.Get("Authorization"); got != "Bearer tok" {
			t.Errorf("Authorization = %q, want %q", got, "Bearer tok")
		}
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"data":[{"user_name":"alice"}]}`))
	}))
	defer srv.Close()

	var out followResp
	if err := newHelixTestClient(srv).Get(context.Background(), "/users/follows", &out); err != nil {
		t.Fatalf("Get: %v", err)
	}
	if len(out.Data) != 1 || out.Data[0].UserName != "alice" {
		t.Fatalf("unexpected body: %+v", out)
	}
}

func TestHelixClient_PostSendsJSONBody(t *testing.T) {
	var gotCT string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		gotCT = r.Header.Get("Content-Type")
		w.WriteHeader(http.StatusAccepted)
	}))
	defer srv.Close()

	body := map[string]string{"k": "v"}
	if err := newHelixTestClient(srv).Post(context.Background(), "/anything", body, nil); err != nil {
		t.Fatalf("Post: %v", err)
	}
	if gotCT != "application/json" {
		t.Errorf("Content-Type = %q, want application/json", gotCT)
	}
}

func TestHelixClient_DeleteWorksWithNilDst(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodDelete {
			t.Fatalf("expected DELETE, got %s", r.Method)
		}
		w.WriteHeader(http.StatusNoContent)
	}))
	defer srv.Close()

	if err := newHelixTestClient(srv).Delete(context.Background(), "/moderation/bans", nil); err != nil {
		t.Fatalf("Delete: %v", err)
	}
}

func TestHelixClient_401ReturnsUnauthorizedSentinel(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		_, _ = w.Write([]byte(`{"error":"Unauthorized","status":401,"message":"Invalid OAuth token"}`))
	}))
	defer srv.Close()

	var out followResp
	err := newHelixTestClient(srv).Get(context.Background(), "/users", &out)
	if err == nil {
		t.Fatal("expected error")
	}
	if !errors.Is(err, ErrUnauthorized) {
		t.Errorf("errors.Is(err, ErrUnauthorized) = false, want true (err=%v)", err)
	}
	var he *HelixError
	if !errors.As(err, &he) {
		t.Fatalf("expected *HelixError, got %T", err)
	}
	if he.Status != 401 || he.Message != "Invalid OAuth token" {
		t.Errorf("HelixError fields = %+v, want Status=401 Message=%q", he, "Invalid OAuth token")
	}
}

func TestHelixClient_NonJSONErrorBodyStillSurfaces(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
		_, _ = w.Write([]byte(`<html>gateway timeout</html>`))
	}))
	defer srv.Close()

	var out followResp
	err := newHelixTestClient(srv).Get(context.Background(), "/anything", &out)
	var he *HelixError
	if !errors.As(err, &he) {
		t.Fatalf("expected *HelixError, got %T: %v", err, err)
	}
	if he.Status != 500 {
		t.Errorf("Status = %d, want 500", he.Status)
	}
	if he.Message == "" {
		t.Error("expected Message to carry raw body on non-JSON error, got empty")
	}
}

func TestHelixClient_429RetriesOnceWithResetHeader(t *testing.T) {
	// Twitch's `Ratelimit-Reset` is in whole Unix seconds, so this test uses
	// a reset value in the past (0) to exercise the retry path without
	// making the test suite wait a full second for the next tick.
	var calls atomic.Int32
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		n := calls.Add(1)
		if n == 1 {
			w.Header().Set("Ratelimit-Reset", "0")
			w.WriteHeader(http.StatusTooManyRequests)
			_, _ = w.Write([]byte(`{"error":"Too Many Requests","status":429}`))
			return
		}
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte(`{"data":[]}`))
	}))
	defer srv.Close()

	var out followResp
	if err := newHelixTestClient(srv).Get(context.Background(), "/ok-after-retry", &out); err != nil {
		t.Fatalf("Get after retry: %v", err)
	}
	if calls.Load() != 2 {
		t.Errorf("expected 2 calls (first 429, then 200), got %d", calls.Load())
	}
}

func TestHelixClient_429TwiceReturnsRateLimited(t *testing.T) {
	// Reset very close to now so the retry fires quickly.
	reset := time.Now().Add(10 * time.Millisecond).Unix()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Ratelimit-Reset", strconv.FormatInt(reset, 10))
		w.WriteHeader(http.StatusTooManyRequests)
	}))
	defer srv.Close()

	var out followResp
	err := newHelixTestClient(srv).Get(context.Background(), "/always-429", &out)
	if err == nil {
		t.Fatal("expected error")
	}
	if !errors.Is(err, ErrRateLimited) {
		t.Errorf("errors.Is(err, ErrRateLimited) = false, want true (err=%v)", err)
	}
}

func TestHelixClient_ContextCancelledDuringRetrySleep(t *testing.T) {
	// Reset 2 seconds out so the wait is always ≥1s after Twitch's Unix-
	// seconds truncation. Cancelling at 30ms reliably interrupts it.
	reset := time.Now().Add(2 * time.Second).Unix()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.Header().Set("Ratelimit-Reset", strconv.FormatInt(reset, 10))
		w.WriteHeader(http.StatusTooManyRequests)
	}))
	defer srv.Close()

	ctx, cancel := context.WithCancel(context.Background())
	go func() {
		time.Sleep(30 * time.Millisecond)
		cancel()
	}()

	var out followResp
	err := newHelixTestClient(srv).Get(ctx, "/slow-reset", &out)
	if !errors.Is(err, context.Canceled) {
		t.Errorf("expected context.Canceled, got %v", err)
	}
}

func TestHelixClient_BaseURLDefaultsWhenEmpty(t *testing.T) {
	// We can't hit the real api.twitch.tv in a test, so we can only verify
	// that a HelixClient with BaseURL="" attempts to resolve the default
	// host. Using an HTTPClient with a Transport that captures the URL
	// without dialing proves the base-url path without network.
	var capturedURL string
	c := &HelixClient{
		HTTPClient: &http.Client{Transport: roundTripFunc(func(r *http.Request) (*http.Response, error) {
			capturedURL = r.URL.String()
			return nil, errSentinelNoopTransport
		})},
		ClientID:    "cid",
		AccessToken: "tok",
	}
	_ = c.Get(context.Background(), "/users", nil)
	if capturedURL != "https://api.twitch.tv/helix/users" {
		t.Errorf("expected default base URL, got %s", capturedURL)
	}
}

var errSentinelNoopTransport = errors.New("noop transport")

type roundTripFunc func(*http.Request) (*http.Response, error)

func (f roundTripFunc) RoundTrip(r *http.Request) (*http.Response, error) { return f(r) }
