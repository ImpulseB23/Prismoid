//go:build linux

package ringbuf

import "fmt"

// Attach is not yet implemented on Linux. See ADR 18 (revised 2026-04-11):
// Linux will use memfd_create + fd passing via a subsequent ticket.
func Attach(_ uintptr, _ int) ([]byte, func(), error) {
	return nil, nil, fmt.Errorf("ring buffer attach not yet supported on linux")
}
