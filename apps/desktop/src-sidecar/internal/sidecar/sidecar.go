// Package sidecar contains the entry-point logic for the Go sidecar process.
//
// The actual main package in cmd/sidecar is a thin shim that calls Run.
// Logic lives here so it can be unit-tested without spawning a real process.
package sidecar

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/rs/zerolog"
	"github.com/rs/zerolog/log"

	"github.com/ImpulseB23/Prismoid/sidecar/internal/control"
	"github.com/ImpulseB23/Prismoid/sidecar/internal/ringbuf"
	"github.com/ImpulseB23/Prismoid/sidecar/internal/twitch"
)

const (
	outChanCapacity = 1024
	cmdChanCapacity = 16
	maxScannerLine  = 1024 * 1024 // 1 MB; EventSub envelopes can exceed the default 64KB
	heartbeatPeriod = 1 * time.Second
)

// Run is the sidecar entry point. It wires real stdin/stdout into RunWithIO,
// which contains the testable lifecycle logic.
func Run() error {
	zerolog.SetGlobalLevel(zerolog.DebugLevel)
	log.Logger = log.Output(zerolog.ConsoleWriter{Out: os.Stderr})

	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer cancel()

	return RunWithIO(ctx, os.Stdin, os.Stdout, log.Logger, ringbuf.Attach)
}

// AttachFunc opens a shared memory section by handle. The production
// implementation is ringbuf.Attach; tests inject a fake.
type AttachFunc func(handle uintptr, size int) ([]byte, func(), error)

// RunWithIO is the testable lifecycle entry: read the bootstrap, attach to
// the shared memory section via the supplied AttachFunc, spawn the writer
// goroutine, and run the command loop until ctx is cancelled or stdin closes.
func RunWithIO(ctx context.Context, stdin io.Reader, stdout io.Writer, logger zerolog.Logger, attach AttachFunc) error {
	logger.Info().Msg("sidecar starting")

	scanner := readerScanner(stdin)

	boot, err := ReadBootstrap(scanner)
	if err != nil {
		logger.Error().Err(err).Msg("failed to read bootstrap")
		return err
	}
	logger.Info().Uint64("handle", uint64(boot.ShmHandle)).Int("size", boot.ShmSize).Msg("bootstrap received")

	mem, cleanup, err := attach(boot.ShmHandle, boot.ShmSize)
	if err != nil {
		logger.Error().Err(err).Msg("failed to attach to shared memory")
		return err
	}
	defer cleanup()

	writer, err := ringbuf.Open(mem)
	if err != nil {
		logger.Error().Err(err).Msg("failed to open ring buffer writer")
		return err
	}

	out := make(chan []byte, outChanCapacity)
	go RunWriter(ctx, out, writer)

	return RunCommandLoop(ctx, scanner, json.NewEncoder(stdout), out, logger)
}

// ReadBootstrap consumes a single line from the scanner and decodes it as a
// control.Bootstrap message. Returns an error on EOF or invalid JSON.
func ReadBootstrap(scanner *bufio.Scanner) (control.Bootstrap, error) {
	if !scanner.Scan() {
		if err := scanner.Err(); err != nil {
			return control.Bootstrap{}, fmt.Errorf("read bootstrap line: %w", err)
		}
		return control.Bootstrap{}, fmt.Errorf("stdin closed before bootstrap")
	}
	var boot control.Bootstrap
	if err := json.Unmarshal(scanner.Bytes(), &boot); err != nil {
		return control.Bootstrap{}, fmt.Errorf("invalid bootstrap message: %w", err)
	}
	return boot, nil
}

// RunWriter is the sole producer to the ring buffer. Multiple platform clients
// send raw envelope bytes via `in`; this goroutine drains them serially. If
// the ring buffer is full it logs and drops, matching the drop-oldest
// backpressure described in docs/architecture.md.
func RunWriter(ctx context.Context, in <-chan []byte, writer *ringbuf.Writer) {
	for {
		select {
		case <-ctx.Done():
			return
		case data := <-in:
			if !writer.Write(data) {
				log.Warn().Msg("ring buffer full, dropping message")
			}
		}
	}
}

// RunCommandLoop drives the heartbeat ticker and command dispatch until ctx is
// cancelled. Reads commands from the scanner via a small fan-in goroutine and
// writes heartbeats + notifications via the encoder.
func RunCommandLoop(ctx context.Context, scanner *bufio.Scanner, encoder *json.Encoder, out chan<- []byte, logger zerolog.Logger) error {
	cmdCh := make(chan control.Command, cmdChanCapacity)
	go scanCommands(scanner, cmdCh, logger)

	heartbeat := time.NewTicker(heartbeatPeriod)
	defer heartbeat.Stop()

	clients := make(map[string]context.CancelFunc)
	notify := makeNotify(encoder, logger)

	for {
		select {
		case <-ctx.Done():
			logger.Info().Msg("sidecar shutting down")
			return nil
		case <-heartbeat.C:
			if err := encoder.Encode(control.Message{Type: "heartbeat"}); err != nil {
				logger.Error().Err(err).Msg("failed to write heartbeat to host")
				return err
			}
		case cmd := <-cmdCh:
			DispatchCommand(ctx, cmd, clients, out, notify, logger)
		}
	}
}

func scanCommands(scanner *bufio.Scanner, cmdCh chan<- control.Command, logger zerolog.Logger) {
	for scanner.Scan() {
		var cmd control.Command
		if err := json.Unmarshal(scanner.Bytes(), &cmd); err != nil {
			logger.Error().Err(err).Msg("invalid command from host")
			continue
		}
		cmdCh <- cmd
	}
}

func makeNotify(encoder *json.Encoder, logger zerolog.Logger) twitch.Notify {
	return func(msgType string, payload any) {
		if err := encoder.Encode(control.Message{Type: msgType, Payload: payload}); err != nil {
			logger.Error().Err(err).Str("type", msgType).Msg("failed to notify host")
		}
	}
}

// DispatchCommand routes a control.Command to its handler.
func DispatchCommand(ctx context.Context, cmd control.Command, clients map[string]context.CancelFunc, out chan<- []byte, notify twitch.Notify, logger zerolog.Logger) {
	switch cmd.Cmd {
	case "twitch_connect":
		HandleTwitchConnect(ctx, cmd, clients, out, notify, logger)
	case "twitch_disconnect":
		HandleTwitchDisconnect(cmd, clients, logger)
	default:
		logger.Info().Str("cmd", cmd.Cmd).Str("channel", cmd.Channel).Msg("received command")
	}
}

// HandleTwitchConnect spawns a Twitch EventSub client for the broadcaster in
// cmd if there isn't already one running. The client writes envelope bytes to
// `out`, which the writer goroutine drains into the ring buffer.
func HandleTwitchConnect(ctx context.Context, cmd control.Command, clients map[string]context.CancelFunc, out chan<- []byte, notify twitch.Notify, logger zerolog.Logger) {
	if _, exists := clients[cmd.BroadcasterID]; exists {
		logger.Warn().Str("broadcaster", cmd.BroadcasterID).Msg("already connected, ignoring")
		return
	}

	clientCtx, clientCancel := context.WithCancel(ctx)

	client := &twitch.Client{
		BroadcasterID: cmd.BroadcasterID,
		UserID:        cmd.UserID,
		AccessToken:   cmd.Token,
		ClientID:      cmd.ClientID,
		Out:           out,
		Log:           logger.With().Str("broadcaster", cmd.BroadcasterID).Logger(),
		Notify:        notify,
	}

	clients[cmd.BroadcasterID] = clientCancel

	go func() {
		if err := client.Run(clientCtx); err != nil && ctx.Err() == nil {
			logger.Error().Err(err).Str("broadcaster", cmd.BroadcasterID).Msg("twitch client exited")
		}
	}()

	logger.Info().Str("broadcaster", cmd.BroadcasterID).Msg("twitch client started")
}

// HandleTwitchDisconnect cancels and removes a previously-connected client.
func HandleTwitchDisconnect(cmd control.Command, clients map[string]context.CancelFunc, logger zerolog.Logger) {
	cancelFn, exists := clients[cmd.BroadcasterID]
	if !exists {
		logger.Warn().Str("broadcaster", cmd.BroadcasterID).Msg("no active connection to disconnect")
		return
	}
	cancelFn()
	delete(clients, cmd.BroadcasterID)
	logger.Info().Str("broadcaster", cmd.BroadcasterID).Msg("twitch client disconnected")
}

// readerScanner is a small helper used by tests; production code constructs
// its scanner directly from os.Stdin in Run.
func readerScanner(r io.Reader) *bufio.Scanner {
	s := bufio.NewScanner(r)
	s.Buffer(make([]byte, 0, maxScannerLine), maxScannerLine)
	return s
}
