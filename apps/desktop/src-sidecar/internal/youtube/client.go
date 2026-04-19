package youtube

import (
	"context"
	"errors"
	"time"

	"github.com/rs/zerolog"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/encoding/protojson"

	"github.com/ImpulseB23/Prismoid/sidecar/internal/backoff"
	"github.com/ImpulseB23/Prismoid/sidecar/internal/control"
	pb "github.com/ImpulseB23/Prismoid/sidecar/internal/youtube/ytpb"
)

const defaultTarget = "dns:///youtube.googleapis.com:443"

// Notify is called on control-plane events that the Rust host should know
// about (auth errors, stream ended). The caller wires this to stdout JSON.
type Notify = func(msgType string, payload any)

// Client streams YouTube live chat messages via the gRPC streamList API and
// writes tagged JSON payloads to a shared channel. The sidecar owns the
// channel and a single writer goroutine drains it into the ring buffer.
type Client struct {
	LiveChatID  string
	APIKey      string // one of APIKey or AccessToken
	AccessToken string

	Target string // override for testing; "" uses default
	// DialOpts replaces the default dial options entirely when set. Tests
	// inject grpc.WithTransportCredentials(insecure.NewCredentials()) here.
	DialOpts []grpc.DialOption

	Out    chan<- []byte
	Log    zerolog.Logger
	Notify Notify
}

// ErrMissingCredentials is returned when neither APIKey nor AccessToken is set.
var ErrMissingCredentials = errors.New("youtube: APIKey or AccessToken required")

// Run connects to the YouTube gRPC streamList endpoint and reads messages
// until ctx is cancelled. Reconnects automatically with exponential backoff.
func (c *Client) Run(ctx context.Context) error {
	if c.APIKey == "" && c.AccessToken == "" {
		if c.Notify != nil {
			c.Notify("auth_error", "youtube: missing api key or access token")
		}
		return ErrMissingCredentials
	}

	bo := backoff.New(1*time.Second, 30*time.Second)

	var pageToken string
	for {
		nextToken, err := c.connectAndStream(ctx, pageToken)
		if err == nil || errors.Is(err, context.Canceled) {
			return err
		}

		// FAILED_PRECONDITION(9) = chat disabled or ended, NOT_FOUND(5) = bad chat ID.
		// Both are permanent; don't retry.
		if s, ok := status.FromError(err); ok {
			switch s.Code() {
			case codes.FailedPrecondition, codes.NotFound:
				c.Log.Warn().Str("code", s.Code().String()).Str("msg", s.Message()).Msg("permanent error, stopping")
				if c.Notify != nil {
					c.Notify("youtube_error", map[string]string{
						"code":    s.Code().String(),
						"message": s.Message(),
					})
				}
				return err
			case codes.PermissionDenied:
				c.Log.Error().Msg("permission denied, check API key / OAuth token")
				if c.Notify != nil {
					c.Notify("auth_error", "youtube permission denied")
				}
				return err
			}
		}

		if nextToken != "" {
			pageToken = nextToken
		}

		c.Log.Warn().Err(err).Msg("youtube stream disconnected, reconnecting")

		delay := bo.Next()
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-time.After(delay):
		}
	}
}

func (c *Client) target() string {
	if c.Target != "" {
		return c.Target
	}
	return defaultTarget
}

func (c *Client) authMetadata() metadata.MD {
	if c.AccessToken != "" {
		return metadata.Pairs("authorization", "Bearer "+c.AccessToken)
	}
	return metadata.Pairs("x-goog-api-key", c.APIKey)
}

func (c *Client) connectAndStream(ctx context.Context, pageToken string) (string, error) {
	opts := []grpc.DialOption{grpc.WithTransportCredentials(credentials.NewClientTLSFromCert(nil, ""))}
	if len(c.DialOpts) > 0 {
		opts = c.DialOpts
	}

	conn, err := grpc.NewClient(c.target(), opts...)
	if err != nil {
		return pageToken, err
	}
	defer func() { _ = conn.Close() }()

	stub := pb.NewV3DataLiveChatMessageServiceClient(conn)

	req := &pb.LiveChatMessageListRequest{
		LiveChatId: &c.LiveChatID,
		Part:       []string{"snippet", "authorDetails"},
	}
	if pageToken != "" {
		req.PageToken = &pageToken
	}

	md := c.authMetadata()
	stream, err := stub.StreamList(metadata.NewOutgoingContext(ctx, md), req)
	if err != nil {
		return pageToken, err
	}

	c.Log.Info().Str("chat_id", c.LiveChatID).Msg("connected to youtube streamList")

	return c.recvLoop(stream, pageToken)
}

var marshaler = protojson.MarshalOptions{UseProtoNames: true}

func (c *Client) recvLoop(stream pb.V3DataLiveChatMessageService_StreamListClient, lastToken string) (string, error) {
	for {
		resp, err := stream.Recv()
		if err != nil {
			return lastToken, err
		}

		if t := resp.GetNextPageToken(); t != "" {
			lastToken = t
		}

		if resp.GetOfflineAt() != "" {
			c.Log.Info().Str("offline_at", resp.GetOfflineAt()).Msg("stream went offline")
			if c.Notify != nil {
				c.Notify("youtube_offline", resp.GetOfflineAt())
			}
		}

		for _, item := range resp.GetItems() {
			js, err := marshaler.Marshal(item)
			if err != nil {
				c.Log.Warn().Err(err).Msg("failed to marshal chat message to json")
				continue
			}

			tagged := make([]byte, 1+len(js))
			tagged[0] = control.TagYouTube
			copy(tagged[1:], js)

			select {
			case c.Out <- tagged:
			default:
				c.Log.Warn().Msg("output channel full, dropping message")
			}
		}
	}
}
