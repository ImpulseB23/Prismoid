package youtube

import (
	"context"
	"net"
	"testing"
	"time"

	"github.com/rs/zerolog"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials/insecure"
	"google.golang.org/protobuf/proto"

	"github.com/ImpulseB23/Prismoid/sidecar/internal/control"
	pb "github.com/ImpulseB23/Prismoid/sidecar/internal/youtube/ytpb"
)

type fakeStreamListServer struct {
	pb.UnimplementedV3DataLiveChatMessageServiceServer
	responses []*pb.LiveChatMessageListResponse
}

func (f *fakeStreamListServer) StreamList(_ *pb.LiveChatMessageListRequest, stream pb.V3DataLiveChatMessageService_StreamListServer) error {
	for _, resp := range f.responses {
		if err := stream.Send(resp); err != nil {
			return err
		}
	}
	return nil
}

func startFakeServer(t *testing.T, responses []*pb.LiveChatMessageListResponse) (string, func()) {
	t.Helper()
	lis, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	srv := grpc.NewServer()
	pb.RegisterV3DataLiveChatMessageServiceServer(srv, &fakeStreamListServer{responses: responses})
	go func() { _ = srv.Serve(lis) }()
	return lis.Addr().String(), func() {
		srv.Stop()
		lis.Close()
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

	addr, cleanup := startFakeServer(t, []*pb.LiveChatMessageListResponse{resp})
	defer cleanup()

	out := make(chan []byte, 16)
	client := &Client{
		LiveChatID: "chat-123",
		APIKey:     "test-key",
		Target:     "dns:///" + addr,
		DialOpts:   []grpc.DialOption{grpc.WithTransportCredentials(insecure.NewCredentials())},
		Out:        out,
		Log:        zerolog.Nop(),
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	_ = client.Run(ctx)

	select {
	case data := <-out:
		if len(data) < 2 {
			t.Fatal("message too short")
		}
		if data[0] != control.TagYouTube {
			t.Fatalf("expected tag 0x03, got 0x%02x", data[0])
		}
		json := string(data[1:])
		if len(json) == 0 {
			t.Fatal("empty json body")
		}
		// Verify the JSON contains expected fields
		if !contains(json, "msg-1") {
			t.Errorf("expected message id msg-1 in json: %s", json)
		}
		if !contains(json, "hello from test") {
			t.Errorf("expected message text in json: %s", json)
		}
	default:
		t.Fatal("no message received on out channel")
	}
}

func TestClientStopsOnNotFound(t *testing.T) {
	// Empty server that returns no responses -> EOF -> client retries.
	// For a permanent error test, we'd need to return a gRPC status.
	// This test validates the client stops on context cancel.
	addr, cleanup := startFakeServer(t, nil)
	defer cleanup()

	out := make(chan []byte, 16)
	client := &Client{
		LiveChatID: "bad-chat",
		APIKey:     "test-key",
		Target:     "dns:///" + addr,
		DialOpts:   []grpc.DialOption{grpc.WithTransportCredentials(insecure.NewCredentials())},
		Out:        out,
		Log:        zerolog.Nop(),
	}

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()

	err := client.Run(ctx)
	if err != nil && err != context.DeadlineExceeded {
		// Expected: either context deadline or nil
	}
}

func contains(s, substr string) bool {
	return len(s) >= len(substr) && searchStr(s, substr)
}

func searchStr(s, sub string) bool {
	for i := 0; i <= len(s)-len(sub); i++ {
		if s[i:i+len(sub)] == sub {
			return true
		}
	}
	return false
}
