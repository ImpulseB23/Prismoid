// Package main is the sidecar binary entry point. All logic lives in
// internal/sidecar so it can be unit-tested without spawning a process.
package main

import (
	"os"

	"github.com/ImpulseB23/Prismoid/sidecar/internal/sidecar"
)

func main() {
	if err := sidecar.Run(); err != nil {
		os.Exit(1)
	}
}
