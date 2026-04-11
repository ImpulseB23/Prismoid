package twitch

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestSubscribeSuccess(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodPost {
			t.Fatalf("expected POST, got %s", r.Method)
		}
		if r.URL.Path != "/eventsub/subscriptions" {
			t.Fatalf("unexpected path: %s", r.URL.Path)
		}
		if r.Header.Get("Authorization") != "Bearer test-token" {
			t.Fatalf("bad auth header: %s", r.Header.Get("Authorization"))
		}
		if r.Header.Get("Client-Id") != "test-client" {
			t.Fatalf("bad client-id: %s", r.Header.Get("Client-Id"))
		}

		var req SubscriptionRequest
		if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
			t.Fatalf("decode body: %v", err)
		}
		if req.Type != "channel.chat.message" {
			t.Fatalf("unexpected type: %s", req.Type)
		}
		if req.Transport.SessionID != "sess-123" {
			t.Fatalf("unexpected session_id: %s", req.Transport.SessionID)
		}

		w.WriteHeader(http.StatusAccepted)
	}))
	defer srv.Close()

	err := Subscribe(context.Background(), srv.URL, "sess-123", "broadcaster-1", "user-1", "test-token", "test-client")
	if err != nil {
		t.Fatalf("expected no error, got: %v", err)
	}
}

func TestSubscribeAuthError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		_, _ = w.Write([]byte(`{"error":"Unauthorized"}`))
	}))
	defer srv.Close()

	err := Subscribe(context.Background(), srv.URL, "sess-123", "b", "u", "bad-token", "client")
	if err == nil {
		t.Fatal("expected error")
	}

	var authErr *AuthError
	if !errors.As(err, &authErr) {
		t.Fatalf("expected AuthError, got %T: %v", err, err)
	}
	if authErr.Status != 401 {
		t.Fatalf("expected status 401, got %d", authErr.Status)
	}
}

func TestSubscribeServerError(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusInternalServerError)
		_, _ = w.Write([]byte(`{"error":"Internal Server Error"}`))
	}))
	defer srv.Close()

	err := Subscribe(context.Background(), srv.URL, "sess-123", "b", "u", "token", "client")
	if err == nil {
		t.Fatal("expected error for 500")
	}

	var authErr *AuthError
	if errors.As(err, &authErr) {
		t.Fatal("500 should not be an AuthError")
	}
}
