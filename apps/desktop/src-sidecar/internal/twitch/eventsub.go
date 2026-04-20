package twitch

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/rs/zerolog"

	"github.com/ImpulseB23/Prismoid/sidecar/internal/backoff"
	"github.com/ImpulseB23/Prismoid/sidecar/internal/control"
)

const defaultWSURL = "wss://eventsub.wss.twitch.tv/ws"

// Notify is called on control-plane events (auth errors, revocations) that
// the Rust host should know about. The caller wires this to stdout JSON.
type Notify func(msgType string, payload any)

// Client streams Twitch EventSub notifications and writes raw envelope bytes
// to a shared channel. The sidecar owns the channel and a single writer
// goroutine drains it into the ring buffer, preserving the SPSC invariant
// across all platform clients.
type Client struct {
	BroadcasterID string
	UserID        string
	ClientID      string
	HelixBase     string // override for testing; "" uses default
	WSURL         string // override for testing; "" uses default

	Out    chan<- []byte
	Log    zerolog.Logger
	Notify Notify

	mu          sync.RWMutex
	accessToken string
}

// NewClient constructs a Client with the given access token. The token
// can be rotated mid-session via SetAccessToken.
func NewClient(broadcasterID, userID, accessToken, clientID string, out chan<- []byte, log zerolog.Logger, notify Notify) *Client {
	return &Client{
		BroadcasterID: broadcasterID,
		UserID:        userID,
		ClientID:      clientID,
		Out:           out,
		Log:           log,
		Notify:        notify,
		accessToken:   accessToken,
	}
}

// AccessToken returns the current token. Safe for concurrent use.
func (c *Client) AccessToken() string {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.accessToken
}

// SetAccessToken rotates the token. The next EventSub reconnect or Helix
// call will pick up the new value.
func (c *Client) SetAccessToken(token string) {
	c.mu.Lock()
	c.accessToken = token
	c.mu.Unlock()
}

// Run connects to EventSub and reads messages until ctx is cancelled.
// It reconnects automatically on errors with exponential backoff.
func (c *Client) Run(ctx context.Context) error {
	bo := backoff.New(1*time.Second, 30*time.Second)
	wsURL := c.WSURL
	if wsURL == "" {
		wsURL = defaultWSURL
	}

	for {
		err := c.connectAndListen(ctx, wsURL)
		if err == nil || errors.Is(err, context.Canceled) {
			return err
		}

		// reconnect messages provide a URL; on normal errors use the default
		var re *reconnectError
		if errors.As(err, &re) {
			wsURL = re.url
			bo.Reset()
			continue
		}

		wsURL = c.wsURL()
		c.Log.Warn().Err(err).Msg("eventsub disconnected, reconnecting")

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
	return defaultWSURL
}

func (c *Client) connectAndListen(ctx context.Context, url string) error {
	conn, _, err := websocket.Dial(ctx, url, nil)
	if err != nil {
		return fmt.Errorf("dial %s: %w", url, err)
	}
	defer func() { _ = conn.CloseNow() }()

	sessionID, keepalive, err := c.readWelcome(ctx, conn)
	if err != nil {
		return err
	}

	c.Log.Info().Str("session", sessionID).Int("keepalive_s", keepalive).Msg("connected to eventsub")

	if err := Subscribe(ctx, c.HelixBase, sessionID, c.BroadcasterID, c.UserID, c.AccessToken(), c.ClientID); err != nil {
		var authErr *AuthError
		if errors.As(err, &authErr) {
			if c.Notify != nil {
				c.Notify("auth_error", authErr.Error())
			}
			return err
		}
		return fmt.Errorf("subscribe: %w", err)
	}

	c.Log.Info().Msg("subscribed to channel.chat.message")

	return c.listenLoop(ctx, conn, keepalive)
}

func (c *Client) readWelcome(ctx context.Context, conn *websocket.Conn) (string, int, error) {
	// twitch expects subscription within keepalive_timeout_seconds (default 10s)
	readCtx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()

	_, data, err := conn.Read(readCtx)
	if err != nil {
		return "", 0, fmt.Errorf("read welcome: %w", err)
	}

	var env Envelope
	if err := json.Unmarshal(data, &env); err != nil {
		return "", 0, fmt.Errorf("unmarshal welcome: %w", err)
	}

	if env.Metadata.MessageType != "session_welcome" {
		return "", 0, fmt.Errorf("expected session_welcome, got %s", env.Metadata.MessageType)
	}

	var payload WelcomePayload
	if err := json.Unmarshal(env.Payload, &payload); err != nil {
		return "", 0, fmt.Errorf("unmarshal welcome payload: %w", err)
	}

	return payload.Session.ID, payload.Session.KeepaliveTimeoutSeconds, nil
}

func (c *Client) listenLoop(ctx context.Context, conn *websocket.Conn, keepaliveSec int) error {
	// Each Read uses a fresh context.WithTimeout, so the keepalive timeout is
	// enforced per read. No standalone timer needed.
	timeout := time.Duration(keepaliveSec)*time.Second + time.Second

	for {
		readCtx, cancel := context.WithTimeout(ctx, timeout)
		_, data, err := conn.Read(readCtx)
		cancel()
		if err != nil {
			if errors.Is(err, context.Canceled) && ctx.Err() != nil {
				_ = conn.Close(websocket.StatusNormalClosure, "shutting down")
				return ctx.Err()
			}
			return fmt.Errorf("read: %w", err)
		}

		var env Envelope
		if err := json.Unmarshal(data, &env); err != nil {
			c.Log.Error().Err(err).Msg("unmarshal envelope failed")
			continue
		}

		switch env.Metadata.MessageType {
		case "notification":
			// Non-blocking send. The receiver (the writer goroutine in the
			// sidecar) is the sole producer to the ring buffer; if its input
			// channel is full it means the ring buffer is also full or close
			// to it. We drop the *new* message (drop-newest) rather than
			// stall the websocket reader. Note: docs/architecture.md
			// describes drop-oldest at the ring buffer layer; the current
			// SPSC primitive cannot evict already-written messages without a
			// reader-side cooperation we haven't built yet. Tracked
			// separately.
			tagged := make([]byte, 1+len(data))
			tagged[0] = control.TagTwitch
			copy(tagged[1:], data)

			select {
			case c.Out <- tagged:
			default:
				c.Log.Warn().Msg("output channel full, dropping message")
			}

		case "session_keepalive":
			// no-op; per-read timeout above resets on every successful Read

		case "session_reconnect":
			var payload ReconnectPayload
			if err := json.Unmarshal(env.Payload, &payload); err != nil {
				c.Log.Error().Err(err).Msg("unmarshal reconnect payload")
				continue
			}
			c.Log.Info().Str("url", payload.Session.ReconnectURL).Msg("reconnect requested")
			return &reconnectError{url: payload.Session.ReconnectURL}

		case "revocation":
			c.Log.Warn().RawJSON("payload", env.Payload).Msg("subscription revoked")
			if c.Notify != nil {
				c.Notify("revocation", json.RawMessage(env.Payload))
			}
			return fmt.Errorf("subscription revoked")

		default:
			c.Log.Debug().Str("type", env.Metadata.MessageType).Msg("unhandled message type")
		}
	}
}

type reconnectError struct {
	url string
}

func (e *reconnectError) Error() string {
	return "reconnect to " + e.url
}
