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
)

func main() {
	zerolog.SetGlobalLevel(zerolog.DebugLevel)
	log.Logger = log.Output(zerolog.ConsoleWriter{Out: os.Stderr})

	ctx, cancel := signal.NotifyContext(context.Background(), syscall.SIGINT, syscall.SIGTERM)
	defer cancel()

	log.Info().Msg("sidecar starting")

	stdin := bufio.NewScanner(os.Stdin)
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
			log.Info().Str("cmd", cmd.Cmd).Str("channel", cmd.Channel).Msg("received command")
		}
	}
}
