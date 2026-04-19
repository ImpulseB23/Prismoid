package youtube

import (
	"context"
	"errors"
	"net"
	"strings"
	"testing"
	"time"

	"github.com/rs/zerolog"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/grpc/status"
	"google.golang.org/protobuf/proto"

	"github.com/ImpulseB23/Prismoid/sidecar/internal/control"
	pb "github.com/ImpulseB23/Prismoid/sidecar/internal/youtube/ytpb"
)

type fakeStreamListServer struct {
	pb.UnimplementedV3DataLiveChatMessageServiceServer
	responses []*pb.LiveChatMessageListResponse
	sendErr   error
}

func (f *fakeStreamListServer) StreamList(_ *pb.LiveChatMessageListRequest, stream pb.V3DataLiveChatMessageService_StreamListServer) error {
	for _, resp := range f.responses {
		if err := stream.Send(resp); err != nil {
			return err
		}
	}
	if f.sendErr != nil {
		return f.sendErr
	}
	<-stream.Context().Done()
	return stream.Context().Err()
}

func startFakeServer(t *testing.T, srvImpl pb.V3DataLiveChatMessageServiceServer) string {
	t.Helper()
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	srv := grpc.NewServer()
	pb.RegisterV3DataLiveChatMessageServiceServer(srv, srvImpl)
	go func() { _ = srv.Serve(lis) }()
	t.Cleanup(func() {
		srv.Stop()
		_ = lis.Close()
	})
	return lis.Addr().String()
}

func newTestClient(addr string, out chan []byte) *Client {
	return &Client{
		LiveChatID: "chat-123",
		APIKey:     "test-key",
		Target:     "dns:///" + addr,
		DialOpts:   []grpc.DialOption{grpc.WithTransportCredentials(insecure.NewCredentials())},
		Out:        out,
		Log:        zerolog.Nop(),
	}
}

func TestClientReceivesTextMessages(t *testing.T) {
	msgType := pb.LiveChatMessageSnippet_TypeWrapper_TEXT_MESSAGE_EVENT
	publishedAt := "2024-06-15T12:30:00Z"
	msgText := "hello from test"
	displayName := "TestUser"
	channelID := "UC_test"

	resp := &pb.LiveChatMessageListResponse{
		Items: []*pb.LiveChatMessage{
			{
				Id: proto.String("msg-1"),
				Snippet: &pb.LiveChatMessageSnippet{
					Type:        &msgType,
					PublishedAt: &publishedAt,
					DisplayedContent: &pb.LiveChatMessageSnippet_TextMessageDetails{
						TextMessageDetails: &pb.LiveChatTextMessageDetails{
							MessageText: &msgText,
						},
					},
				},
				AuthorDetails: &pb.LiveChatMessageAuthorDetails{
					ChannelId:   &channelID,
					DisplayName: &displayName,
				},
			},
		},
	}

	addr := startFakeServer(t, &fakeStreamListServer{responses: []*pb.LiveChatMessageListResponse{resp}})

	out := make(chan []byte, 16)
	client := newTestClient(addr, out)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	done := make(chan error, 1)
	go func() { done <- client.Run(ctx) }()

	select {
	case data := <-out:
		cancel()
		if len(data) < 2 {
			t.Fatal("message too short")
		}
		if data[0] != control.TagYouTube {
			t.Fatalf("expected tag 0x03, got 0x%02x", data[0])
		}
		body := string(data[1:])
		if !strings.Contains(body, "msg-1") {
			t.Errorf("expected message id msg-1 in json: %s", body)
		}
		if !strings.Contains(body, "hello from test") {
			t.Errorf("expected message text in json: %s", body)
		}
	case <-ctx.Done():
		t.Fatal("timed out waiting for message")
	}

	select {
	case <-done:
	case <-time.After(2 * time.Second):
		t.Fatal("client.Run did not exit after cancel")
	}
}

func TestClientStopsOnNotFound(t *testing.T) {
	srv := &fakeStreamListServer{sendErr: status.Error(codes.NotFound, "chat not found")}
	addr := startFakeServer(t, srv)

	out := make(chan []byte, 1)
	client := newTestClient(addr, out)

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	err := client.Run(ctx)
	if err == nil {
		t.Fatal("expected NotFound error, got nil")
	}
	s, ok := status.FromError(err)
	if !ok {
		t.Fatalf("expected gRPC status error, got %T: %v", err, err)
	}
	if s.Code() != codes.NotFound {
		t.Fatalf("expected NotFound, got %s", s.Code())
	}
}

func TestClientRequiresCredentials(t *testing.T) {
	out := make(chan []byte, 1)
	client := &Client{
		LiveChatID: "chat-123",
		Out:        out,
		Log:        zerolog.Nop(),
	}
	err := client.Run(context.Background())
	if !errors.Is(err, ErrMissingCredentials) {
		t.Fatalf("expected ErrMissingCredentials, got %v", err)
	}
}
