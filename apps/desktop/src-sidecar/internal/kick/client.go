package kick

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strconv"
	"time"

	"github.com/coder/websocket"
	"github.com/rs/zerolog"

	"github.com/ImpulseB23/Prismoid/sidecar/internal/backoff"
	"github.com/ImpulseB23/Prismoid/sidecar/internal/control"
)

const (
	defaultPusherURL = "wss://ws-us2.pusher.com/app/32cbd69e4b950bf97679?protocol=7&client=js&version=8.4.0-rc2&flash=false"
	channelPrefix    = "chatrooms."
	channelSuffix    = ".v2"
)

// Notify is called on control-plane events that the Rust host should know
// about. Mirrors twitch.Notify.
type Notify func(msgType string, payload any)

// Client streams Kick chat messages from a Pusher WebSocket channel and writes
// raw Pusher event bytes (tagged with control.TagKick) to a shared channel.
// Follows the same lifecycle pattern as the Twitch EventSub client.
type Client struct {
	ChatroomID  int
	WSURL       string        // override for testing; "" uses default
	PongTimeout time.Duration // override for testing; 0 uses default (30s)

	Out    chan<- []byte
	Log    zerolog.Logger
	Notify Notify
}

// Run connects to the Kick Pusher WebSocket and reads messages until ctx is
// cancelled. Reconnects automatically with exponential backoff on errors.
// Pusher close codes 4000-4099 are fatal (do not reconnect).
func (c *Client) Run(ctx context.Context) error {
	bo := backoff.New(1*time.Second, 30*time.Second)

	for {
		err := c.connectAndListen(ctx)
		if err == nil || errors.Is(err, context.Canceled) {
			return err
		}

		if isFatalClose(err) {
			c.Log.Error().Err(err).Msg("fatal pusher close, not reconnecting")
			return err
		}

		c.Log.Warn().Err(err).Msg("kick disconnected, reconnecting")

		delay := bo.Next()
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(delay):
		}
	}
}

func (c *Client) wsURL() string {
	if c.WSURL != "" {
		return c.WSURL
	}
	return defaultPusherURL
}

func (c *Client) connectAndListen(ctx context.Context) error {
	conn, _, err := websocket.Dial(ctx, c.wsURL(), nil)
	if err != nil {
		return fmt.Errorf("dial %s: %w", c.wsURL(), err)
	}
	defer func() { _ = conn.CloseNow() }()

	activityTimeout, err := c.readConnectionEstablished(ctx, conn)
	if err != nil {
		return err
	}

	c.Log.Info().Int("chatroom", c.ChatroomID).Int("activity_timeout", activityTimeout).Msg("connected to kick pusher")

	if err := c.subscribe(ctx, conn); err != nil {
		return err
	}

	return c.listenLoop(ctx, conn, activityTimeout)
}

func (c *Client) readConnectionEstablished(ctx context.Context, conn *websocket.Conn) (int, error) {
	readCtx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()

	_, data, err := conn.Read(readCtx)
	if err != nil {
		return 0, fmt.Errorf("read connection_established: %w", err)
	}

	var ev PusherEvent
	if err := json.Unmarshal(data, &ev); err != nil {
		return 0, fmt.Errorf("unmarshal connection_established: %w", err)
	}
	if ev.Event != "pusher:connection_established" {
		return 0, fmt.Errorf("expected pusher:connection_established, got %s", ev.Event)
	}

	var connData ConnectionData
	if err := json.Unmarshal([]byte(ev.Data), &connData); err != nil {
		return 0, fmt.Errorf("unmarshal connection data: %w", err)
	}

	return connData.ActivityTimeout, nil
}

func (c *Client) subscribe(ctx context.Context, conn *websocket.Conn) error {
	channel := channelPrefix + strconv.Itoa(c.ChatroomID) + channelSuffix
	sub := PusherEvent{
		Event: "pusher:subscribe",
		Data:  `{"channel":"` + channel + `"}`,
	}

	msg, err := json.Marshal(sub)
	if err != nil {
		return fmt.Errorf("marshal subscribe: %w", err)
	}

	writeCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()

	if err := conn.Write(writeCtx, websocket.MessageText, msg); err != nil {
		return fmt.Errorf("write subscribe: %w", err)
	}

	c.Log.Info().Str("channel", channel).Msg("subscribed to kick chatroom")
	return nil
}

func (c *Client) listenLoop(ctx context.Context, conn *websocket.Conn, activityTimeoutSec int) error {
	pingInterval := time.Duration(activityTimeoutSec) * time.Second
	pongTimeout := c.PongTimeout
	if pongTimeout == 0 {
		pongTimeout = 30 * time.Second
	}

	type readResult struct {
		data []byte
		err  error
	}

	// Read goroutine: reads messages and sends them to results.
	// Exits when conn is closed (via deferred CloseNow) or ctx is cancelled.
	results := make(chan readResult, 1)
	loopDone := make(chan struct{})
	defer close(loopDone)
	defer func() { _ = conn.CloseNow() }()

	go func() {
		for {
			_, data, err := conn.Read(ctx)
			select {
			case results <- readResult{data, err}:
			case <-loopDone:
				return
			}
			if err != nil {
				return
			}
		}
	}()

	timer := time.NewTimer(pingInterval)
	defer timer.Stop()
	waitingForPong := false

	for {
		select {
		case r := <-results:
			if r.err != nil {
				if ctx.Err() != nil {
					return ctx.Err()
				}
				return fmt.Errorf("read: %w", r.err)
			}

			if !timer.Stop() {
				select {
				case <-timer.C:
				default:
				}
			}
			timer.Reset(pingInterval)
			waitingForPong = false

			var ev PusherEvent
			if err := json.Unmarshal(r.data, &ev); err != nil {
				c.Log.Error().Err(err).Msg("unmarshal pusher event failed")
				continue
			}

			switch {
			case ev.Event == "pusher:ping":
				if err := c.sendPong(ctx, conn); err != nil {
					return err
				}

			case ev.Event == "pusher:pong":
				// pong received, waitingForPong already cleared above

			case ev.Event == "pusher:error":
				c.Log.Warn().Str("data", ev.Data).Msg("pusher error")

			case ev.Event == "pusher_internal:subscription_succeeded":
				// no-op

			case ev.Channel != "":
				tagged := make([]byte, 1+len(r.data))
				tagged[0] = control.TagKick
				copy(tagged[1:], r.data)
				select {
				case c.Out <- tagged:
				default:
					c.Log.Warn().Msg("output channel full, dropping message")
				}

			default:
				c.Log.Debug().Str("event", ev.Event).Msg("unhandled pusher event")
			}

		case <-timer.C:
			if waitingForPong {
				return fmt.Errorf("pong timeout after %s", pongTimeout)
			}
			if err := c.sendPing(ctx, conn); err != nil {
				return err
			}
			waitingForPong = true
			timer.Reset(pongTimeout)
		}
	}
}

func (c *Client) sendPing(ctx context.Context, conn *websocket.Conn) error {
	msg, _ := json.Marshal(PusherEvent{Event: "pusher:ping", Data: "{}"})
	writeCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	if err := conn.Write(writeCtx, websocket.MessageText, msg); err != nil {
		return fmt.Errorf("write ping: %w", err)
	}
	return nil
}

func (c *Client) sendPong(ctx context.Context, conn *websocket.Conn) error {
	msg, _ := json.Marshal(PusherEvent{Event: "pusher:pong", Data: "{}"})
	writeCtx, cancel := context.WithTimeout(ctx, 5*time.Second)
	defer cancel()
	if err := conn.Write(writeCtx, websocket.MessageText, msg); err != nil {
		return fmt.Errorf("write pong: %w", err)
	}
	return nil
}

// isFatalClose checks if the error contains a Pusher close code in the
// 4000-4099 range, which means "do not reconnect".
func isFatalClose(err error) bool {
	var closeErr websocket.CloseError
	if errors.As(err, &closeErr) {
		code := int(closeErr.Code)
		return code >= 4000 && code <= 4099
	}
	return false
}
