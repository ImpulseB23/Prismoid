package main

import (
	"bufio"
	"context"
	"encoding/json"
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

const outChanCapacity = 1024

func main() {
	zerolog.SetGlobalLevel(zerolog.DebugLevel)
	log.Logger = log.Output(zerolog.ConsoleWriter{Out: os.Stderr})

	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer cancel()

	log.Info().Msg("sidecar starting")

	stdin := bufio.NewScanner(os.Stdin)
	// EventSub envelopes can be larger than the default 64KB scanner buffer.
	stdin.Buffer(make([]byte, 0, 1024*1024), 1024*1024)

	// Phase 1: read the bootstrap line. The Rust host writes this once at
	// spawn, before any commands. It hands over the inherited shared memory
	// section so we can attach without naming a kernel object.
	if !stdin.Scan() {
		if err := stdin.Err(); err != nil {
			log.Error().Err(err).Msg("failed to read bootstrap line")
		} else {
			log.Error().Msg("stdin closed before bootstrap")
		}
		return
	}

	var boot control.Bootstrap
	if err := json.Unmarshal(stdin.Bytes(), &boot); err != nil {
		log.Error().Err(err).Msg("invalid bootstrap message")
		return
	}

	log.Info().Uint64("handle", uint64(boot.ShmHandle)).Int("size", boot.ShmSize).Msg("bootstrap received")

	mem, cleanup, err := ringbuf.Attach(boot.ShmHandle, boot.ShmSize)
	if err != nil {
		log.Error().Err(err).Msg("failed to attach to shared memory")
		return
	}
	defer cleanup()

	writer, err := ringbuf.Open(mem)
	if err != nil {
		log.Error().Err(err).Msg("failed to open ring buffer writer")
		return
	}

	// One channel feeds the sole writer of the ring buffer. Every platform
	// client sends raw envelope bytes here; the writer goroutine serializes
	// them into the ring. SPSC invariant preserved by construction.
	out := make(chan []byte, outChanCapacity)
	go runWriter(ctx, out, writer)

	// Phase 2: command loop. The same scanner reads subsequent commands.
	cmdCh := make(chan control.Command, 16)
	go func() {
		for stdin.Scan() {
			var cmd control.Command
			if err := json.Unmarshal(stdin.Bytes(), &cmd); err != nil {
				log.Error().Err(err).Msg("invalid command from host")
				continue
			}
			cmdCh <- cmd
		}
	}()

	heartbeat := time.NewTicker(1 * time.Second)
	defer heartbeat.Stop()

	encoder := json.NewEncoder(os.Stdout)
	clients := make(map[string]context.CancelFunc)

	notify := func(msgType string, payload any) {
		if err := encoder.Encode(control.Message{Type: msgType, Payload: payload}); err != nil {
			log.Error().Err(err).Str("type", msgType).Msg("failed to notify host")
		}
	}

	for {
		select {
		case <-ctx.Done():
			log.Info().Msg("sidecar shutting down")
			return
		case <-heartbeat.C:
			if err := encoder.Encode(control.Message{Type: "heartbeat"}); err != nil {
				log.Error().Err(err).Msg("failed to write heartbeat to host")
				return
			}
		case cmd := <-cmdCh:
			switch cmd.Cmd {
			case "twitch_connect":
				handleTwitchConnect(ctx, cmd, clients, out, notify)
			case "twitch_disconnect":
				handleTwitchDisconnect(cmd, clients)
			default:
				log.Info().Str("cmd", cmd.Cmd).Str("channel", cmd.Channel).Msg("received command")
			}
		}
	}
}

// runWriter is the sole producer to the ring buffer. Multiple platform clients
// send raw envelope bytes via `out`; this goroutine drains them serially. If
// the ring buffer is full it logs and drops, matching the drop-oldest
// backpressure described in docs/architecture.md.
func runWriter(ctx context.Context, out <-chan []byte, writer *ringbuf.Writer) {
	for {
		select {
		case <-ctx.Done():
			return
		case data := <-out:
			if !writer.Write(data) {
				log.Warn().Msg("ring buffer full, dropping message")
			}
		}
	}
}

func handleTwitchConnect(ctx context.Context, cmd control.Command, clients map[string]context.CancelFunc, out chan<- []byte, notify twitch.Notify) {
	if _, exists := clients[cmd.BroadcasterID]; exists {
		log.Warn().Str("broadcaster", cmd.BroadcasterID).Msg("already connected, ignoring")
		return
	}

	clientCtx, clientCancel := context.WithCancel(ctx)

	client := &twitch.Client{
		BroadcasterID: cmd.BroadcasterID,
		UserID:        cmd.UserID,
		AccessToken:   cmd.Token,
		ClientID:      cmd.ClientID,
		Out:           out,
		Log:           log.With().Str("broadcaster", cmd.BroadcasterID).Logger(),
		Notify:        notify,
	}

	clients[cmd.BroadcasterID] = clientCancel

	go func() {
		if err := client.Run(clientCtx); err != nil && ctx.Err() == nil {
			log.Error().Err(err).Str("broadcaster", cmd.BroadcasterID).Msg("twitch client exited")
		}
	}()

	log.Info().Str("broadcaster", cmd.BroadcasterID).Msg("twitch client started")
}

func handleTwitchDisconnect(cmd control.Command, clients map[string]context.CancelFunc) {
	cancelFn, exists := clients[cmd.BroadcasterID]
	if !exists {
		log.Warn().Str("broadcaster", cmd.BroadcasterID).Msg("no active connection to disconnect")
		return
	}
	cancelFn()
	delete(clients, cmd.BroadcasterID)
	log.Info().Str("broadcaster", cmd.BroadcasterID).Msg("twitch client disconnected")
}
