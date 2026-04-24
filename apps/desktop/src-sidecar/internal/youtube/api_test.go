package youtube

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func newAPIClient(srv *httptest.Server) *APIClient {
	return &APIClient{
		HTTPClient:  srv.Client(),
		BaseURL:     srv.URL,
		AccessToken: "tok",
	}
}

func TestSendChatMessageSuccess(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			t.Errorf("method = %s, want POST", r.Method)
		}
		if got := r.URL.Path; got != "/liveChat/messages" {
			t.Errorf("path = %s, want /liveChat/messages", got)
		}
		if got := r.URL.Query().Get("part"); got != "snippet" {
			t.Errorf("part = %s, want snippet", got)
		}
		if got := r.Header.Get("Authorization"); got != "Bearer tok" {
			t.Errorf("authorization = %s", got)
		}
		var req SendChatMessageRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Fatalf("decode: %v", err)
		}
		if req.Snippet.LiveChatID != "abc" {
			t.Errorf("liveChatId = %s", req.Snippet.LiveChatID)
		}
		if req.Snippet.Type != "textMessageEvent" {
			t.Errorf("type = %s", req.Snippet.Type)
		}
		if req.Snippet.TextMessageDetails.MessageText != "hi there" {
			t.Errorf("messageText = %s", req.Snippet.TextMessageDetails.MessageText)
		}
		w.WriteHeader(http.StatusOK)
		_, _ = io.WriteString(w, `{"id":"yt-msg-1"}`)
	}))
	defer srv.Close()

	resp, err := newAPIClient(srv).SendChatMessage(context.Background(), "abc", "hi there")
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if resp.ID != "yt-msg-1" {
		t.Errorf("id = %s, want yt-msg-1", resp.ID)
	}
}

func TestSendChatMessageEmptyMessage(t *testing.T) {
	c := &APIClient{AccessToken: "tok"}
	if _, err := c.SendChatMessage(context.Background(), "abc", ""); err == nil {
		t.Fatal("expected error on empty message")
	}
}

func TestSendChatMessageMissingChatID(t *testing.T) {
	c := &APIClient{AccessToken: "tok"}
	if _, err := c.SendChatMessage(context.Background(), "", "hi"); err == nil {
		t.Fatal("expected error on empty liveChatId")
	}
}

func TestSendChatMessageOversize(t *testing.T) {
	c := &APIClient{AccessToken: "tok"}
	big := strings.Repeat("a", MaxMessageRunes+1)
	if _, err := c.SendChatMessage(context.Background(), "abc", big); err == nil {
		t.Fatal("expected error on oversized message")
	}
}

func TestSendChatMessageUnauthorized(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		_, _ = io.WriteString(w, `{"error":{"code":401,"message":"invalid creds","errors":[{"reason":"authError"}]}}`)
	}))
	defer srv.Close()

	_, err := newAPIClient(srv).SendChatMessage(context.Background(), "abc", "hi")
	if err == nil {
		t.Fatal("expected error")
	}
	if !errors.Is(err, ErrUnauthorized) {
		t.Errorf("expected ErrUnauthorized, got %v", err)
	}
	var apiErr *APIError
	if !errors.As(err, &apiErr) {
		t.Fatalf("expected *APIError, got %T", err)
	}
	if apiErr.Reason != "authError" {
		t.Errorf("reason = %s, want authError", apiErr.Reason)
	}
}

func TestSendChatMessageQuotaExceeded(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusForbidden)
		_, _ = io.WriteString(w, `{"error":{"code":403,"message":"quota","errors":[{"reason":"quotaExceeded"}]}}`)
	}))
	defer srv.Close()

	_, err := newAPIClient(srv).SendChatMessage(context.Background(), "abc", "hi")
	if !errors.Is(err, ErrQuotaExceeded) {
		t.Errorf("expected ErrQuotaExceeded, got %v", err)
	}
}

func TestSendChatMessageNonJSONErrorBody(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusBadGateway)
		_, _ = io.WriteString(w, `<html>upstream broke</html>`)
	}))
	defer srv.Close()

	_, err := newAPIClient(srv).SendChatMessage(context.Background(), "abc", "hi")
	var apiErr *APIError
	if !errors.As(err, &apiErr) {
		t.Fatalf("expected *APIError, got %T", err)
	}
	if !strings.Contains(apiErr.Message, "upstream broke") {
		t.Errorf("raw body not surfaced: %s", apiErr.Message)
	}
}
