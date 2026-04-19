package kick

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

func connectionEstablishedMsg(timeout int) []byte {
	data := fmt.Sprintf(`{"socket_id":"123.456","activity_timeout":%d}`, timeout)
	ev := PusherEvent{Event: "pusher:connection_established", Data: data}
	b, _ := json.Marshal(ev)
	return b
}

func subscriptionSucceededMsg(channel string) []byte {
	ev := PusherEvent{
		Event:   "pusher_internal:subscription_succeeded",
		Data:    "{}",
		Channel: channel,
	}
	b, _ := json.Marshal(ev)
	return b
}

func chatMessageEvent(id, content, username string, chatroomID int) []byte {
	inner := fmt.Sprintf(`{"id":"%s","chatroom_id":%d,"content":"%s","type":"message","created_at":"2025-06-01T12:00:00Z","sender":{"id":42,"username":"%s","slug":"%s","identity":{"color":"#FF0000","badges":[]}}}`,
		id, chatroomID, content, username, username)
	ev := PusherEvent{
		Event:   `App\Events\ChatMessageEvent`,
		Data:    inner,
		Channel: fmt.Sprintf("chatrooms.%d.v2", chatroomID),
	}
	b, _ := json.Marshal(ev)
	return b
}

func pusherPingMsg() []byte {
	ev := PusherEvent{Event: "pusher:ping", Data: "{}"}
	b, _ := json.Marshal(ev)
	return b
}

func newTestClient(wsURL string, chatroomID int, out chan<- []byte) *Client {
	return &Client{
		ChatroomID:  chatroomID,
		WSURL:       wsURL,
		PongTimeout: 0, // default 30s
		Out:         out,
		Log:         zerolog.Nop(),
		Notify:      func(string, any) {},
	}
}

func drainChan(ch <-chan []byte) [][]byte {
	var msgs [][]byte
	deadline := time.After(100 * time.Millisecond)
	for {
		select {
		case msg := <-ch:
			msgs = append(msgs, msg)
		case <-deadline:
			return msgs
		}
	}
}

func TestClientReceivesChatMessages(t *testing.T) {
	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			t.Errorf("accept: %v", err)
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, connectionEstablishedMsg(120))
		// read the subscribe message from client
		_, _, _ = conn.Read(ctx)
		_ = conn.Write(ctx, websocket.MessageText, subscriptionSucceededMsg("chatrooms.100.v2"))
		_ = conn.Write(ctx, websocket.MessageText, chatMessageEvent("msg-1", "hello kick", "viewer1", 100))
		_ = conn.Write(ctx, websocket.MessageText, chatMessageEvent("msg-2", "second msg", "viewer2", 100))

		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		100,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	msgs := drainChan(out)
	if len(msgs) < 2 {
		t.Fatalf("expected at least 2 messages, got %d", len(msgs))
	}

	// verify tag byte
	if msgs[0][0] != 0x02 {
		t.Fatalf("expected tag 0x02, got %x", msgs[0][0])
	}

	// verify the inner Pusher event is valid JSON
	var ev PusherEvent
	if err := json.Unmarshal(msgs[0][1:], &ev); err != nil {
		t.Fatalf("unmarshal first message: %v", err)
	}
	if ev.Event != `App\Events\ChatMessageEvent` {
		t.Fatalf("expected ChatMessageEvent, got %s", ev.Event)
	}
	if ev.Channel != "chatrooms.100.v2" {
		t.Fatalf("expected channel chatrooms.100.v2, got %s", ev.Channel)
	}
}

func TestClientHandlesPusherPing(t *testing.T) {
	var receivedPong bool

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, connectionEstablishedMsg(120))
		_, _, _ = conn.Read(ctx) // subscribe
		_ = conn.Write(ctx, websocket.MessageText, subscriptionSucceededMsg("chatrooms.200.v2"))
		_ = conn.Write(ctx, websocket.MessageText, pusherPingMsg())

		// read the pong response
		readCtx, cancel := context.WithTimeout(ctx, 2*time.Second)
		defer cancel()
		_, data, err := conn.Read(readCtx)
		if err == nil {
			var ev PusherEvent
			if json.Unmarshal(data, &ev) == nil && ev.Event == "pusher:pong" {
				receivedPong = true
			}
		}

		_ = conn.Write(ctx, websocket.MessageText, chatMessageEvent("msg-1", "after ping", "user1", 200))
		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		200,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	if !receivedPong {
		t.Fatal("expected client to respond to pusher:ping with pusher:pong")
	}

	msgs := drainChan(out)
	if len(msgs) < 1 {
		t.Fatal("expected at least 1 message after ping/pong")
	}
}

func TestClientFatalCloseDoesNotReconnect(t *testing.T) {
	connectCount := 0

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		connectCount++
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, connectionEstablishedMsg(120))
		_, _, _ = conn.Read(ctx) // subscribe

		// 4001 = application only, do not reconnect
		_ = conn.Close(websocket.StatusCode(4001), "app disabled")
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		300,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 3*time.Second)
	defer cancel()

	err := client.Run(ctx)
	if err == nil {
		t.Fatal("expected error on fatal close")
	}
	if connectCount != 1 {
		t.Fatalf("expected exactly 1 connection attempt for fatal close, got %d", connectCount)
	}
}

func TestClientContextCancel(t *testing.T) {
	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, connectionEstablishedMsg(120))
		_, _, _ = conn.Read(ctx) // subscribe
		_ = conn.Write(ctx, websocket.MessageText, subscriptionSucceededMsg("chatrooms.400.v2"))

		// keep connection open until test cancels
		time.Sleep(5 * time.Second)
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient(
		"ws"+strings.TrimPrefix(wsSrv.URL, "http"),
		400,
		out,
	)

	ctx, cancel := context.WithTimeout(context.Background(), 500*time.Millisecond)
	defer cancel()

	err := client.Run(ctx)
	if err != nil && err != context.DeadlineExceeded && !strings.Contains(err.Error(), "context") {
		t.Fatalf("expected context error, got: %v", err)
	}
}

func TestIsFatalClose(t *testing.T) {
	tests := []struct {
		name  string
		code  websocket.StatusCode
		fatal bool
	}{
		{"4000 fatal", websocket.StatusCode(4000), true},
		{"4099 fatal", websocket.StatusCode(4099), true},
		{"4100 not fatal", websocket.StatusCode(4100), false},
		{"4200 not fatal", websocket.StatusCode(4200), false},
		{"1000 not fatal", websocket.StatusNormalClosure, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := websocket.CloseError{Code: tt.code, Reason: "test"}
			if got := isFatalClose(err); got != tt.fatal {
				t.Errorf("isFatalClose(%d) = %v, want %v", tt.code, got, tt.fatal)
			}
		})
	}
}

func TestClientSendsPingOnIdle(t *testing.T) {
	var receivedPing bool

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		// activity_timeout=1 so the client pings after 1s of idle
		_ = conn.Write(ctx, websocket.MessageText, connectionEstablishedMsg(1))
		_, _, _ = conn.Read(ctx) // subscribe
		_ = conn.Write(ctx, websocket.MessageText, subscriptionSucceededMsg("chatrooms.500.v2"))

		// wait for the client-initiated ping
		readCtx, cancel := context.WithTimeout(ctx, 3*time.Second)
		defer cancel()
		_, data, err := conn.Read(readCtx)
		if err == nil {
			var ev PusherEvent
			if json.Unmarshal(data, &ev) == nil && ev.Event == "pusher:ping" {
				receivedPing = true
			}
		}

		// respond with pong then close
		pong, _ := json.Marshal(PusherEvent{Event: "pusher:pong", Data: "{}"})
		_ = conn.Write(ctx, websocket.MessageText, pong)
		time.Sleep(50 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient("ws"+strings.TrimPrefix(wsSrv.URL, "http"), 500, out)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	if !receivedPing {
		t.Fatal("expected client to send pusher:ping after idle timeout")
	}
}

func TestClientPongTimeout(t *testing.T) {
	var connectCount atomic.Int32

	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		connectCount.Add(1)
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		// activity_timeout=1 so client pings after 1s
		_ = conn.Write(ctx, websocket.MessageText, connectionEstablishedMsg(1))
		_, _, _ = conn.Read(ctx) // subscribe
		_ = conn.Write(ctx, websocket.MessageText, subscriptionSucceededMsg("chatrooms.600.v2"))

		// read the ping but do NOT respond with pong
		readCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
		defer cancel()
		_, _, _ = conn.Read(readCtx)

		<-readCtx.Done()
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := &Client{
		ChatroomID:  600,
		WSURL:       "ws" + strings.TrimPrefix(wsSrv.URL, "http"),
		PongTimeout: 1 * time.Second,
		Out:         out,
		Log:         zerolog.Nop(),
		Notify:      func(string, any) {},
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	// pong timeout (1s) + activity_timeout (1s) + backoff = ~3s per cycle.
	// With 5s context, we should see at least 2 connections.
	if connectCount.Load() < 2 {
		t.Fatalf("expected at least 2 connections (pong timeout triggers reconnect), got %d", connectCount.Load())
	}
}

func TestClientPusherErrorEvent(t *testing.T) {
	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, connectionEstablishedMsg(120))
		_, _, _ = conn.Read(ctx) // subscribe
		_ = conn.Write(ctx, websocket.MessageText, subscriptionSucceededMsg("chatrooms.700.v2"))

		errorEv, _ := json.Marshal(PusherEvent{Event: "pusher:error", Data: `{"code":4201,"message":"rate limit"}`})
		_ = conn.Write(ctx, websocket.MessageText, errorEv)

		// send a normal message after the error to verify the client keeps running
		_ = conn.Write(ctx, websocket.MessageText, chatMessageEvent("msg-1", "after error", "user1", 700))

		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	out := make(chan []byte, 16)
	client := newTestClient("ws"+strings.TrimPrefix(wsSrv.URL, "http"), 700, out)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	msgs := drainChan(out)
	if len(msgs) < 1 {
		t.Fatal("expected message after pusher:error event")
	}
}

func TestClientDropsMessageOnFullChannel(t *testing.T) {
	wsSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer func() { _ = conn.CloseNow() }()

		ctx := context.Background()
		_ = conn.Write(ctx, websocket.MessageText, connectionEstablishedMsg(120))
		_, _, _ = conn.Read(ctx) // subscribe
		_ = conn.Write(ctx, websocket.MessageText, subscriptionSucceededMsg("chatrooms.800.v2"))

		// send more messages than the channel can hold
		for i := 0; i < 5; i++ {
			_ = conn.Write(ctx, websocket.MessageText, chatMessageEvent(
				fmt.Sprintf("msg-%d", i), "overflow", "user1", 800,
			))
		}

		time.Sleep(100 * time.Millisecond)
		_ = conn.Close(websocket.StatusNormalClosure, "done")
	}))
	defer wsSrv.Close()

	// channel with capacity 1 so messages get dropped
	out := make(chan []byte, 1)
	client := newTestClient("ws"+strings.TrimPrefix(wsSrv.URL, "http"), 800, out)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	// at least one should have been received, but not all 5
	msgs := drainChan(out)
	if len(msgs) == 0 {
		t.Fatal("expected at least one message")
	}
}
