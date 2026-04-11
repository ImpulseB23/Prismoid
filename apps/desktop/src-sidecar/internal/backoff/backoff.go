package backoff

import (
	"math"
	"math/rand/v2"
	"time"
)

// Backoff implements exponential backoff with full jitter.
// See https://aws.amazon.com/blogs/architecture/exponential-backoff-and-jitter/
type Backoff struct {
	base    time.Duration
	max     time.Duration
	attempt int
}

func New(base, max time.Duration) *Backoff {
	return &Backoff{base: base, max: max}
}

// Next returns a jittered duration in [0, min(max, base * 2^attempt)] and increments the attempt.
func (b *Backoff) Next() time.Duration {
	ceiling := float64(b.base) * math.Pow(2, float64(b.attempt))
	if ceiling > float64(b.max) {
		ceiling = float64(b.max)
	}
	b.attempt++
	return time.Duration(rand.Int64N(int64(ceiling) + 1))
}

func (b *Backoff) Reset() {
	b.attempt = 0
}
