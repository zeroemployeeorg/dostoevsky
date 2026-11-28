//go:build ignore

// Writes the vectors that check the Rust test seam against the Go one (internal/seam):
//
//	go run rust/testdata/seam_vectors.go > rust/testdata/seam-go.txt
//
// Run from the repository root. Each line is one step and what Go returned: "fix START STEP SEED", "now NANOSECONDS",
// "advance NANOSECONDS", or "random N HEX".
package main

import (
	"encoding/hex"
	"fmt"
	"runtime"
	"time"

	"github.com/rodriveracom/dostoevsky/internal/seam"
)

func main() {
	fmt.Printf("# written by %s with rust/testdata/seam_vectors.go; do not edit\n", runtime.Version())
	now := func() { fmt.Printf("now %d\n", seam.Now().UnixNano()) }
	random := func(n int) {
		p := make([]byte, n)
		if _, err := seam.Random(p); err != nil {
			panic(err)
		}
		fmt.Printf("random %d %s\n", n, hex.EncodeToString(p))
	}
	advance := func(d int64) {
		seam.Advance(time.Duration(d))
		fmt.Printf("advance %d\n", d)
	}
	for _, f := range []struct {
		start, step int64
		seed        uint64
	}{{1790000000000000000, 1000, 1}, {0, 1, 0}, {-5, 7, 1<<64 - 1}, {1 << 40, 0, 0x0123456789abcdef}} {
		seam.Fix(f.start, f.step, f.seed)
		fmt.Printf("fix %d %d %d\n", f.start, f.step, f.seed)
		now()
		now()
		random(0)
		random(1)
		random(16)
		random(32)
		random(33)
		advance(1_000_000_007)
		now()
		random(100)
		advance(-3)
		now()
		random(5)
		random(64)
		now()
	}
	seam.Restore()
}
