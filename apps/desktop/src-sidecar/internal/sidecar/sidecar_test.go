package sidecar

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"
	"unsafe"

	"github.com/rs/zerolog"

	"github.com/ImpulseB23/Prismoid/sidecar/internal/control"
	"github.com/ImpulseB23/Prismoid/sidecar/internal/ringbuf"
)

const (
	testCacheLine  = 64
	testHeaderSize = testCacheLine * 3
)

// makeTestRingBuffer constructs a ring buffer writer backed by a plain []byte.
// The header layout matches the production ringbuf package; this is the same
// pattern PR #24's tests used before they were rewritten to use channels.
func makeTestRingBuffer(t *testing.T, capacity int) (*ringbuf.Writer, []byte) {
	t.Helper()
	mem := make([]byte, testHeaderSize+capacity)
	*(*uint64)(unsafe.Pointer(&mem[testCacheLine*2])) = uint64(capacity)
	w, err := ringbuf.Open(mem)
	if err != nil {
		t.Fatalf("ringbuf.Open: %v", err)
	}
	return w, mem
}

func TestReadBootstrap_Valid(t *testing.T) {
	r := strings.NewReader(`{"shm_handle": 12345, "shm_size": 4096}` + "\n")
	scanner := readerScanner(r)

	boot, err := ReadBootstrap(scanner)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if boot.ShmHandle != 12345 {
		t.Errorf("expected handle 12345, got %d", boot.ShmHandle)
	}
	if boot.ShmSize != 4096 {
		t.Errorf("expected size 4096, got %d", boot.ShmSize)
	}
}

func TestReadBootstrap_InvalidJSON(t *testing.T) {
	r := strings.NewReader("not json\n")
	scanner := readerScanner(r)

	_, err := ReadBootstrap(scanner)
	if err == nil {
		t.Fatal("expected error for invalid json")
	}
	if !strings.Contains(err.Error(), "invalid bootstrap message") {
		t.Errorf("expected wrapped error message, got: %v", err)
	}
}

func TestReadBootstrap_EOF(t *testing.T) {
	r := strings.NewReader("")
	scanner := readerScanner(r)

	_, err := ReadBootstrap(scanner)
	if err == nil {
		t.Fatal("expected error for EOF before bootstrap")
	}
	if !strings.Contains(err.Error(), "stdin closed") {
		t.Errorf("expected stdin closed message, got: %v", err)
	}
}

func TestRunWriter_DrainsChannelToRing(t *testing.T) {
	writer, mem := makeTestRingBuffer(t, 4096)

	in := make(chan []byte, 4)
	in <- []byte("hello")
	in <- []byte("world")

	var signalCount atomic.Int32
	signal := func() { signalCount.Add(1) }

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		RunWriter(ctx, in, writer, signal)
		close(done)
	}()

	// give the goroutine a moment to drain
	time.Sleep(50 * time.Millisecond)
	cancel()
	<-done

	// inspect the ring buffer header to verify writes happened
	writePos := *(*uint64)(unsafe.Pointer(&mem[0]))
	// "hello" = 5 + 4 length prefix; "world" = 5 + 4 length prefix; total 18 bytes
	if writePos != 18 {
		t.Errorf("expected write index 18, got %d", writePos)
	}

	// each successful write should have signaled exactly once
	if got := signalCount.Load(); got != 2 {
		t.Errorf("expected 2 signals, got %d", got)
	}
}

func TestRunWriter_StopsOnContextCancel(t *testing.T) {
	writer, _ := makeTestRingBuffer(t, 4096)

	in := make(chan []byte, 1)
	ctx, cancel := context.WithCancel(context.Background())

	done := make(chan struct{})
	go func() {
		RunWriter(ctx, in, writer, func() {})
		close(done)
	}()

	cancel()
	select {
	case <-done:
	case <-time.After(time.Second):
		t.Fatal("RunWriter did not exit on context cancel")
	}
}

func TestRunWriter_SkipsSignalOnFullRing(t *testing.T) {
	// Tiny 32-byte data region; each message is 4 bytes framing + 20 bytes
	// payload = 24 bytes. Second write should fail (capacity 32 < 48).
	writer, _ := makeTestRingBuffer(t, 32)

	in := make(chan []byte, 4)
	in <- make([]byte, 20)
	in <- make([]byte, 20)

	var signalCount atomic.Int32
	signal := func() { signalCount.Add(1) }

	ctx, cancel := context.WithCancel(context.Background())
	done := make(chan struct{})
	go func() {
		RunWriter(ctx, in, writer, signal)
		close(done)
	}()

	time.Sleep(50 * time.Millisecond)
	cancel()
	<-done

	// Only the first write succeeded, so signal should have fired exactly once.
	if got := signalCount.Load(); got != 1 {
		t.Errorf("expected 1 signal (dropped writes must not signal), got %d", got)
	}
}

func TestHandleTwitchConnect_AddsClient(t *testing.T) {
	clients := make(map[string]context.CancelFunc)
	out := make(chan []byte, 1)
	cmd := control.Command{
		Cmd:           "twitch_connect",
		BroadcasterID: "broadcaster-1",
		UserID:        "user-1",
		Token:         "tok",
		ClientID:      "cid",
	}

	HandleTwitchConnect(context.Background(), cmd, clients, out, func(string, any) {}, zerolog.Nop())

	if _, ok := clients["broadcaster-1"]; !ok {
		t.Fatal("expected client to be registered")
	}

	// clean up the goroutine the handler spawned
	clients["broadcaster-1"]()
}

func TestHandleTwitchConnect_RejectsDuplicate(t *testing.T) {
	clients := make(map[string]context.CancelFunc)
	out := make(chan []byte, 1)

	var cancelled atomic.Bool
	clients["broadcaster-1"] = func() { cancelled.Store(true) }

	cmd := control.Command{Cmd: "twitch_connect", BroadcasterID: "broadcaster-1"}
	HandleTwitchConnect(context.Background(), cmd, clients, out, func(string, any) {}, zerolog.Nop())

	if cancelled.Load() {
		t.Fatal("existing client cancel was overwritten")
	}
	if len(clients) != 1 {
		t.Fatalf("expected 1 client, got %d", len(clients))
	}
}

func TestHandleTwitchDisconnect_CancelsAndRemoves(t *testing.T) {
	clients := make(map[string]context.CancelFunc)
	var cancelled atomic.Bool
	clients["broadcaster-1"] = func() { cancelled.Store(true) }

	cmd := control.Command{Cmd: "twitch_disconnect", BroadcasterID: "broadcaster-1"}
	HandleTwitchDisconnect(cmd, clients, zerolog.Nop())

	if !cancelled.Load() {
		t.Fatal("expected client to be cancelled")
	}
	if _, ok := clients["broadcaster-1"]; ok {
		t.Fatal("expected client to be removed from registry")
	}
}

func TestHandleTwitchDisconnect_NoOpForUnknown(t *testing.T) {
	clients := make(map[string]context.CancelFunc)
	cmd := control.Command{Cmd: "twitch_disconnect", BroadcasterID: "broadcaster-unknown"}

	// should not panic
	HandleTwitchDisconnect(cmd, clients, zerolog.Nop())
}

func TestDispatchCommand_RoutesConnect(t *testing.T) {
	clients := make(map[string]context.CancelFunc)
	out := make(chan []byte, 1)
	cmd := control.Command{Cmd: "twitch_connect", BroadcasterID: "b1"}

	DispatchCommand(context.Background(), cmd, clients, out, func(string, any) {}, zerolog.Nop())

	if _, ok := clients["b1"]; !ok {
		t.Fatal("expected connect to register client")
	}
	clients["b1"]()
}

func TestDispatchCommand_RoutesDisconnect(t *testing.T) {
	var cancelled atomic.Bool
	clients := map[string]context.CancelFunc{
		"b1": func() { cancelled.Store(true) },
	}
	out := make(chan []byte, 1)
	cmd := control.Command{Cmd: "twitch_disconnect", BroadcasterID: "b1"}

	DispatchCommand(context.Background(), cmd, clients, out, func(string, any) {}, zerolog.Nop())

	if !cancelled.Load() {
		t.Fatal("expected disconnect to cancel client")
	}
}

func TestDispatchCommand_IgnoresUnknown(t *testing.T) {
	clients := make(map[string]context.CancelFunc)
	out := make(chan []byte, 1)
	cmd := control.Command{Cmd: "frobnicate"}

	// should not panic, should not modify clients
	DispatchCommand(context.Background(), cmd, clients, out, func(string, any) {}, zerolog.Nop())

	if len(clients) != 0 {
		t.Fatalf("expected no clients, got %d", len(clients))
	}
}

func TestRunCommandLoop_DispatchesCommands(t *testing.T) {
	// Pre-load the scanner with one twitch_connect, then close stdin so
	// scanCommands exits cleanly. The loop itself stops when ctx is cancelled.
	stdin := strings.NewReader(`{"cmd":"twitch_connect","broadcaster_id":"b1"}` + "\n")
	scanner := readerScanner(stdin)

	var stdout bytes.Buffer
	encoder := json.NewEncoder(&stdout)

	out := make(chan []byte, 1)
	ctx, cancel := context.WithCancel(context.Background())

	done := make(chan error, 1)
	go func() { done <- RunCommandLoop(ctx, scanner, encoder, out, zerolog.Nop()) }()

	// give the loop a moment to dispatch the queued command
	time.Sleep(150 * time.Millisecond)
	cancel()

	select {
	case err := <-done:
		if err != nil {
			t.Fatalf("RunCommandLoop returned error: %v", err)
		}
	case <-time.After(2 * time.Second):
		t.Fatal("RunCommandLoop did not exit")
	}
}

func TestRunCommandLoop_EmitsHeartbeat(t *testing.T) {
	// No commands; let the heartbeat fire at least once.
	scanner := readerScanner(strings.NewReader(""))

	var stdout bytes.Buffer
	encoder := json.NewEncoder(&stdout)

	out := make(chan []byte, 1)
	ctx, cancel := context.WithTimeout(context.Background(), heartbeatPeriod+200*time.Millisecond)
	defer cancel()

	if err := RunCommandLoop(ctx, scanner, encoder, out, zerolog.Nop()); err != nil {
		t.Fatalf("RunCommandLoop returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), `"type":"heartbeat"`) {
		t.Errorf("expected at least one heartbeat in stdout, got: %s", stdout.String())
	}
}

// errWriter forces encoder.Encode to fail so we can exercise the heartbeat
// error return path.
type errWriter struct{ err error }

func (w *errWriter) Write(_ []byte) (int, error) { return 0, w.err }

func TestRunCommandLoop_PropagatesHeartbeatWriteError(t *testing.T) {
	scanner := readerScanner(strings.NewReader(""))
	encoder := json.NewEncoder(&errWriter{err: errors.New("pipe broken")})

	out := make(chan []byte, 1)
	ctx, cancel := context.WithTimeout(context.Background(), heartbeatPeriod+200*time.Millisecond)
	defer cancel()

	err := RunCommandLoop(ctx, scanner, encoder, out, zerolog.Nop())
	if err == nil {
		t.Fatal("expected error when encoder.Encode fails")
	}
}

func TestMakeNotify_EncodesMessage(t *testing.T) {
	var buf bytes.Buffer
	var mu sync.Mutex
	notify := makeNotify(json.NewEncoder(&buf), &mu, zerolog.Nop())
	notify("auth_error", "expired token")

	var msg control.Message
	if err := json.Unmarshal(buf.Bytes(), &msg); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if msg.Type != "auth_error" {
		t.Errorf("expected auth_error, got %s", msg.Type)
	}
	if msg.Payload != "expired token" {
		t.Errorf("expected payload, got %v", msg.Payload)
	}
}

func TestMakeNotify_SerializesConcurrentWrites(t *testing.T) {
	// Hammers notify from many goroutines and verifies the resulting stream
	// is a sequence of well-formed JSON messages, not interleaved garbage.
	// Without the mutex, json.Encoder.Encode calls would race on the shared
	// io.Writer and produce torn output.
	var buf bytes.Buffer
	var mu sync.Mutex
	notify := makeNotify(json.NewEncoder(&buf), &mu, zerolog.Nop())

	const goroutines = 16
	const perGoroutine = 50
	var wg sync.WaitGroup
	for g := 0; g < goroutines; g++ {
		wg.Add(1)
		go func(id int) {
			defer wg.Done()
			for i := 0; i < perGoroutine; i++ {
				notify("auth_error", id)
			}
		}(g)
	}
	wg.Wait()

	dec := json.NewDecoder(&buf)
	count := 0
	for {
		var msg control.Message
		if err := dec.Decode(&msg); err != nil {
			break
		}
		count++
	}
	if count != goroutines*perGoroutine {
		t.Errorf("expected %d well-formed messages, got %d", goroutines*perGoroutine, count)
	}
}

// nopNotify is a stub NotifyFunc used by RunWithIO tests that don't care
// about signaling. Returns nil without side effects.
func nopNotify(_ uintptr) error { return nil }

func TestMakeSignalFunc_NoopsOnZeroEventHandle(t *testing.T) {
	var called atomic.Bool
	notify := func(_ uintptr) error {
		called.Store(true)
		return nil
	}
	signal := MakeSignalFunc(0, notify, zerolog.Nop())
	signal()
	if called.Load() {
		t.Fatal("notify should not be called when eventHandle is 0")
	}
}

func TestMakeSignalFunc_CallsNotifyWithEventHandle(t *testing.T) {
	var got atomic.Uintptr
	notify := func(h uintptr) error {
		got.Store(h)
		return nil
	}
	signal := MakeSignalFunc(0x1234, notify, zerolog.Nop())
	signal()
	if got.Load() != 0x1234 {
		t.Fatalf("expected notify called with 0x1234, got %#x", got.Load())
	}
}

func TestMakeSignalFunc_SwallowsNotifyError(t *testing.T) {
	// A notify that returns an error must not panic or block. The error is
	// logged at Warn level and the writer goroutine continues.
	notify := func(_ uintptr) error {
		return errors.New("set event failed")
	}
	signal := MakeSignalFunc(0x42, notify, zerolog.Nop())
	// Must not panic.
	signal()
}

func TestRunWithIO_AttachError(t *testing.T) {
	// Bootstrap is valid; the AttachFunc fails. RunWithIO must propagate.
	stdin := strings.NewReader(`{"shm_handle": 1, "shm_event_handle": 2, "shm_size": 4096}` + "\n")
	var stdout bytes.Buffer

	fakeAttach := func(uintptr, int) ([]byte, func(), error) {
		return nil, nil, errors.New("attach failed")
	}

	err := RunWithIO(context.Background(), stdin, &stdout, zerolog.Nop(), fakeAttach, nopNotify)
	if err == nil || err.Error() != "attach failed" {
		t.Fatalf("expected attach failed error, got: %v", err)
	}
}

func TestRunWithIO_BootstrapError(t *testing.T) {
	stdin := strings.NewReader("not json\n")
	var stdout bytes.Buffer

	err := RunWithIO(context.Background(), stdin, &stdout, zerolog.Nop(), nil, nopNotify)
	if err == nil {
		t.Fatal("expected bootstrap error")
	}
	if !strings.Contains(err.Error(), "invalid bootstrap message") {
		t.Errorf("expected invalid bootstrap error, got: %v", err)
	}
}

func TestRunWithIO_RingbufOpenError(t *testing.T) {
	// Bootstrap valid, attach returns an undersized buffer that ringbuf.Open
	// rejects. RunWithIO must propagate that error.
	stdin := strings.NewReader(`{"shm_handle": 1, "shm_event_handle": 2, "shm_size": 8}` + "\n")
	var stdout bytes.Buffer

	fakeAttach := func(uintptr, int) ([]byte, func(), error) {
		return make([]byte, 8), func() {}, nil
	}

	err := RunWithIO(context.Background(), stdin, &stdout, zerolog.Nop(), fakeAttach, nopNotify)
	if err == nil {
		t.Fatal("expected ringbuf.Open error for undersized buffer")
	}
}

func TestScanCommands_SkipsInvalidJSON(t *testing.T) {
	stdin := strings.NewReader("not json\n" + `{"cmd":"twitch_disconnect","broadcaster_id":"b1"}` + "\n")
	scanner := readerScanner(stdin)
	cmdCh := make(chan control.Command, 4)

	scanCommands(scanner, cmdCh, zerolog.Nop())

	// only the valid command should make it through
	select {
	case cmd := <-cmdCh:
		if cmd.Cmd != "twitch_disconnect" {
			t.Errorf("expected twitch_disconnect, got %s", cmd.Cmd)
		}
	default:
		t.Fatal("expected one valid command on the channel")
	}

	select {
	case extra := <-cmdCh:
		t.Fatalf("unexpected extra command: %+v", extra)
	default:
	}
}

func TestMakeNotify_LogsOnEncoderError(t *testing.T) {
	encoder := json.NewEncoder(&errWriter{err: errors.New("pipe broken")})
	var mu sync.Mutex
	notify := makeNotify(encoder, &mu, zerolog.Nop())
	// must not panic; error path is exercised internally
	notify("auth_error", "anything")
}

func TestRunWithIO_HappyPath(t *testing.T) {
	// Bootstrap valid, attach returns a real buffer, command loop runs until
	// the context is cancelled. The writer goroutine is spawned and must drain
	// gracefully on shutdown.
	stdin := strings.NewReader(`{"shm_handle": 1, "shm_event_handle": 2, "shm_size": 4096}` + "\n")
	var stdout bytes.Buffer

	mem := make([]byte, testHeaderSize+4096)
	*(*uint64)(unsafe.Pointer(&mem[testCacheLine*2])) = uint64(4096)

	fakeAttach := func(uintptr, int) ([]byte, func(), error) {
		return mem, func() {}, nil
	}

	ctx, cancel := context.WithTimeout(context.Background(), heartbeatPeriod+200*time.Millisecond)
	defer cancel()

	if err := RunWithIO(ctx, stdin, &stdout, zerolog.Nop(), fakeAttach, nopNotify); err != nil {
		t.Fatalf("RunWithIO returned error: %v", err)
	}

	if !strings.Contains(stdout.String(), `"type":"heartbeat"`) {
		t.Errorf("expected at least one heartbeat in stdout, got: %s", stdout.String())
	}
}
