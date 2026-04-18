package twitch

import (
	"context"
	"encoding/json"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/coder/websocket"
	"github.com/rs/zerolog"
)

func welcomeMsg(sessionID string, keepaliveSec int) []byte {
	msg := fmt.Sprintf(`{
		"metadata":{"message_id":"1","message_type":"session_welcome","message_timestamp":"2025-01-01T00:00:00Z"},
		"payload":{"session":{"id":"%s","keepalive_timeout_seconds":%d}}
	}`, sessionID, keepaliveSec)
	return []byte(msg)
}

func notificationMsg(id, text string) []byte {
	msg := fmt.Sprintf(`{
		"metadata":{"message_id":"%s","message_type":"notification","message_timestamp":"2025-01-01T00:00:01Z"},
		"payload":{"subscription":{"type":"channel.chat.message"},"event":{"message":{"text":"%s"}}}
	}`, id, text)
	return []byte(msg)
}

func keepaliveMsg() []byte {
	return []byte(`{"metadata":{"message_id":"ka-1","message_type":"session_keepalive","message_timestamp":"2025-01-01T00:00:02Z"},"payload":{}}`)
}

func newTestClient(wsURL, helixURL string, out chan<- []byte) *Client {
	return &Client{
		BroadcasterID: "broadcaster-1",
		UserID:        "user-1",
		AccessToken:   "test-token",
		ClientID:      "test-client",
		HelixBase:     helixURL,
		WSURL:         wsURL,
		Out:           out,
		Log:           zerolog.Nop(),
		Notify:        func(string, any) {},
	}
}

// drainChan reads everything currently in ch (non-blocking after a brief
// settle) so tests can assert on the bytes the client sent. The settle gives
// the client a chance to flush after Run returns from the test server closing.
func drainChan(ch <-chan []byte) [][]byte {
	var msgs [][]byte
	deadline := time.After(50 * time.Millisecond)
	for {
		select {
		case msg := <-ch:
			msgs = append(msgs, msg)
		case <-deadline:
			return msgs
		}
	}
}

func TestClientReceivesNotifications(t *testing.T) {
	helixSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusAccepted)
	}))
	defer helixSrv.Close()

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			t.Errorf("accept: %v", err)
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, welcomeMsg("sess-1", 30))
		_ = conn.Write(ctx, websocket.MessageText, notificationMsg("n-1", "hello chat"))
		_ = conn.Write(ctx, websocket.MessageText, notificationMsg("n-2", "second msg"))

		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		helixSrv.URL,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	msgs := drainChan(out)
	if len(msgs) < 2 {
		t.Fatalf("expected at least 2 messages on the out channel, got %d", len(msgs))
	}

	var env Envelope
	if msgs[0][0] != 0x01 {
		t.Fatalf("expected tag 0x01, got %x", msgs[0][0])
	}
	if err := json.Unmarshal(msgs[0][1:], &env); err != nil {
		t.Fatalf("unmarshal first message: %v", err)
	}
	if env.Metadata.MessageType != "notification" {
		t.Fatalf("expected notification, got %s", env.Metadata.MessageType)
	}
}

func TestClientHandlesKeepalive(t *testing.T) {
	helixSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusAccepted)
	}))
	defer helixSrv.Close()

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, welcomeMsg("sess-2", 30))
		_ = conn.Write(ctx, websocket.MessageText, keepaliveMsg())
		_ = conn.Write(ctx, websocket.MessageText, notificationMsg("n-1", "after keepalive"))

		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		helixSrv.URL,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	msgs := drainChan(out)
	if len(msgs) < 1 {
		t.Fatal("expected at least 1 notification after keepalive")
	}
}

func TestClientAuthError(t *testing.T) {
	helixSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusUnauthorized)
		_, _ = w.Write([]byte(`{"error":"Unauthorized"}`))
	}))
	defer helixSrv.Close()

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, welcomeMsg("sess-3", 30))

		time.Sleep(2 * time.Second)
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)

	var notified atomic.Bool
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		helixSrv.URL,
		out,
	)
	client.Notify = func(msgType string, _ any) {
		if msgType == "auth_error" {
			notified.Store(true)
		}
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	err := client.Run(ctx)
	if err == nil {
		t.Fatal("expected error on auth failure")
	}
	if !notified.Load() {
		t.Fatal("expected auth_error notification")
	}
}

func TestClientReconnect(t *testing.T) {
	helixSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusAccepted)
	}))
	defer helixSrv.Close()

	var secondConnected atomic.Bool

	// second server (reconnect target)
	secondSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()
		secondConnected.Store(true)

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, welcomeMsg("sess-new", 30))
		_ = conn.Write(ctx, websocket.MessageText, notificationMsg("n-after-reconnect", "reconnected"))

		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer secondSrv.Close()

	reconnectURL := "ws" + strings.TrimPrefix(secondSrv.URL, "http")

	// first server sends reconnect
	firstSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, welcomeMsg("sess-old", 30))

		reconnectMsg := fmt.Sprintf(`{
			"metadata":{"message_id":"r-1","message_type":"session_reconnect","message_timestamp":"2025-01-01T00:00:03Z"},
			"payload":{"session":{"id":"sess-old","reconnect_url":"%s"}}
		}`, reconnectURL)
		_ = conn.Write(ctx, websocket.MessageText, []byte(reconnectMsg))

		time.Sleep(2 * time.Second)
	}))
	defer firstSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient(
		"ws"+strings.TrimPrefix(firstSrv.URL, "http"),
		helixSrv.URL,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	if !secondConnected.Load() {
		t.Fatal("expected client to connect to reconnect URL")
	}

	msgs := drainChan(out)
	if len(msgs) < 1 {
		t.Fatal("expected at least 1 message after reconnect")
	}
}

func TestClientRevocation(t *testing.T) {
	helixSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusAccepted)
	}))
	defer helixSrv.Close()

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, welcomeMsg("sess-rev", 30))
		_ = conn.Write(ctx, websocket.MessageText, []byte(`{
			"metadata":{"message_id":"rev-1","message_type":"revocation","message_timestamp":"2025-01-01T00:00:04Z"},
			"payload":{"subscription":{"type":"channel.chat.message","status":"user_removed"}}
		}`))

		time.Sleep(2 * time.Second)
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)

	var notifyType atomic.Value
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		helixSrv.URL,
		out,
	)
	client.Notify = func(msgType string, _ any) {
		notifyType.Store(msgType)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	err := client.Run(ctx)
	if err == nil {
		t.Fatal("expected error on revocation")
	}

	v := notifyType.Load()
	if v == nil || v.(string) != "revocation" {
		t.Fatalf("expected revocation notification, got %v", v)
	}
}

func TestClientChannelFull(t *testing.T) {
	helixSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusAccepted)
	}))
	defer helixSrv.Close()

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, welcomeMsg("sess-full", 30))

		// send many notifications without anyone draining the channel
		for i := range 20 {
			_ = conn.Write(ctx, websocket.MessageText, notificationMsg(
				fmt.Sprintf("n-%d", i),
				"will eventually overflow the tiny channel",
			))
		}

		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	// tiny channel that fills almost immediately; the client must drop
	// without blocking the websocket reader.
	out := make(chan []byte, 1)
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		helixSrv.URL,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	// should not panic, just drop messages
	_ = client.Run(ctx)
}

func TestReconnectErrorMessage(t *testing.T) {
	e := &reconnectError{url: "wss://example.com/new"}
	got := e.Error()
	if got != "reconnect to wss://example.com/new" {
		t.Fatalf("unexpected error message: %s", got)
	}
}

func TestClient_wsURLDefault(t *testing.T) {
	c := &Client{}
	if got := c.wsURL(); got != defaultWSURL {
		t.Errorf("expected default %q, got %q", defaultWSURL, got)
	}
}

func TestClient_wsURLOverride(t *testing.T) {
	c := &Client{WSURL: "wss://override.example/ws"}
	if got := c.wsURL(); got != "wss://override.example/ws" {
		t.Errorf("expected override, got %q", got)
	}
}

func TestClientHandlesUnknownMessageType(t *testing.T) {
	helixSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusAccepted)
	}))
	defer helixSrv.Close()

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, welcomeMsg("sess-unknown", 30))
		_ = conn.Write(ctx, websocket.MessageText, []byte(`{"metadata":{"message_id":"u-1","message_type":"future_type","message_timestamp":"2025-01-01T00:00:05Z"},"payload":{}}`))
		_ = conn.Write(ctx, websocket.MessageText, notificationMsg("n-after-unknown", "still working"))

		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		helixSrv.URL,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	// the unknown type message must not break the loop; we should still see
	// the notification that came after it
	msgs := drainChan(out)
	if len(msgs) < 1 {
		t.Fatal("expected the post-unknown notification to make it through")
	}
}

func TestClientHandlesMalformedEnvelope(t *testing.T) {
	helixSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusAccepted)
	}))
	defer helixSrv.Close()

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, welcomeMsg("sess-malformed", 30))
		_ = conn.Write(ctx, websocket.MessageText, []byte(`not json at all`))
		_ = conn.Write(ctx, websocket.MessageText, notificationMsg("n-after-bad", "still alive"))

		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		helixSrv.URL,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	msgs := drainChan(out)
	if len(msgs) < 1 {
		t.Fatal("expected the post-malformed notification to make it through")
	}
}
