package backoff

import (
	"testing"
	"time"
)

func TestNextStaysWithinBounds(t *testing.T) {
	b := New(1*time.Second, 30*time.Second)
	for i := range 20 {
		d := b.Next()
		if d < 0 {
			t.Fatalf("attempt %d: got negative duration %v", i, d)
		}
		if d > 30*time.Second {
			t.Fatalf("attempt %d: %v exceeds max 30s", i, d)
		}
	}
}

func TestCeilingGrows(t *testing.T) {
	b := New(1*time.Second, 1*time.Hour)

	var maxSeen time.Duration
	for range 100 {
		d := b.Next()
		if d > maxSeen {
			maxSeen = d
		}
	}

	// with 100 attempts and a 1h cap, we should see values well above 1s
	if maxSeen < 10*time.Second {
		t.Fatalf("expected to see values above 10s across 100 attempts, max was %v", maxSeen)
	}
}

func TestReset(t *testing.T) {
	b := New(1*time.Second, 30*time.Second)
	for range 10 {
		b.Next()
	}
	b.Reset()

	// after reset, ceiling should be back to base (1s), so values must be <= 1s
	for range 50 {
		d := b.Next()
		if d > 1*time.Second {
			t.Fatalf("after reset, first attempt exceeded base: %v", d)
		}
		b.Reset()
	}
}

func TestCapRespected(t *testing.T) {
	b := New(100*time.Millisecond, 500*time.Millisecond)
	for range 50 {
		d := b.Next()
		if d > 500*time.Millisecond {
			t.Fatalf("exceeded cap: %v", d)
		}
	}
}
