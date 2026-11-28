//go:build ignore

// Writes the vectors that check the Rust wire codec against package wire:
//
//	go run rust/testdata/wire_vectors.go > rust/testdata/wire-go.txt
//
// Run from the repository root. Every frame of wire/testdata/vectors.json is damaged in many small ways, and each
// result is given to the Go decoders. One line per input and decoder: the decoder ("Q" request, "P" reply, "L" hello,
// "W" welcome), the input in hex, Go's verdict, and the fields Go decoded. Frames of megabytes, and the encoders, are
// in wire_more_vectors.go.
package main

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"math/rand"
	"os"
	"runtime"

	"github.com/rodriveracom/dostoevsky/wire"
)

func verdict(err error) string {
	var refused *wire.RequestError
	switch {
	case err == nil:
		return "ok"
	case errors.As(err, &refused):
		return fmt.Sprintf("refuse:%d", refused.Code)
	case errors.Is(err, wire.ErrTooLong):
		return "toolong"
	case errors.Is(err, wire.ErrProtocol):
		return "protocol"
	case err == io.EOF:
		return "eof"
	case err == io.ErrUnexpectedEOF:
		return "unexpected"
	}
	panic(err)
}

func x(b []byte) string { return "-" + hex.EncodeToString(b) + "-" }

func main() {
	raw, err := os.ReadFile("wire/testdata/vectors.json")
	if err != nil {
		panic(err)
	}
	var file struct {
		Valid, Invalid []struct{ Hex string }
	}
	if err := json.Unmarshal(raw, &file); err != nil {
		panic(err)
	}
	rng := rand.New(rand.NewSource(20261128))
	seen := map[string]bool{}
	var inputs [][]byte
	add := func(b []byte) {
		if !seen[string(b)] {
			seen[string(b)] = true
			inputs = append(inputs, bytes.Clone(b))
		}
	}
	// refit makes the header's length agree with the body again, so that damage reaches the body rules.
	refit := func(b []byte) []byte {
		b = bytes.Clone(b)
		if len(b) >= wire.HeaderSize {
			binary.LittleEndian.PutUint32(b, uint32(len(b)-wire.HeaderSize))
		}
		return b
	}
	for _, v := range append(file.Valid, file.Invalid...) {
		b, err := hex.DecodeString(v.Hex)
		if err != nil {
			panic(err)
		}
		add(b)
		// A long frame is damaged at every byte of its header and at 48 places in its body: enough to reach each
		// rule without a row for every byte.
		places := make([]int, len(b))
		for i := range places {
			places[i] = i
		}
		if len(b) > 200 {
			places = places[:wire.HeaderSize]
			for len(places) < wire.HeaderSize+48 {
				places = append(places, wire.HeaderSize+rng.Intn(len(b)-wire.HeaderSize))
			}
		}
		for _, i := range places {
			c := bytes.Clone(b)
			c[i] ^= byte(1 << rng.Intn(8))
			add(c)
			add(refit(c))
			c[i] = []byte{0x00, 0xff, 0x80, 0x7f}[rng.Intn(4)]
			add(c)
			add(b[:i])
			add(refit(b[:i]))
		}
		for n := 0; n < 12; n++ {
			c := bytes.Clone(b)
			for k := 1 + rng.Intn(3); k > 0 && len(c) > 0; k-- {
				c[rng.Intn(len(c))] = byte(rng.Intn(256))
			}
			if rng.Intn(3) == 0 {
				c = append(c, byte(rng.Intn(256)))
			}
			add(c)
			add(refit(c))
		}
		add(append(bytes.Clone(b), 0))
		add(refit(append(bytes.Clone(b), 0)))
	}
	fmt.Printf("# written by %s with rust/testdata/wire_vectors.go; do not edit\n", runtime.Version())
	for _, b := range inputs {
		h, herr := wire.ParseHeader(b)
		if herr != nil {
			fmt.Printf("H %s %s\n", x(b), verdict(herr))
		} else {
			q, err := wire.DecodeRequest(h, b[wire.HeaderSize:])
			fmt.Printf("Q %s %s %d %d %s %d %d %d %s %s %s %s\n", x(b), verdict(err), q.Type, q.RequestID, x([]byte(q.Topic)), q.Partition, q.Offset, q.WaitMS,
				x([]byte(q.Group)), x([]byte(q.Key)), x([]byte(q.MessageID)), x(q.Value))
			p, err := wire.DecodeReply(h, b[wire.HeaderSize:])
			fmt.Printf("P %s %s %d %d %d %d %t %d %s %s %s %d\n", x(b), verdict(err), p.Type, p.RequestID, p.Partition, p.Offset, p.Found, p.Time,
				x([]byte(p.Key)), x([]byte(p.MessageID)), x(p.Value), p.Code)
		}
		// The handshake decoders refuse nearly every frame at its first bytes: keep those lines for one input in eight.
		handshake := bytes.HasPrefix(b, []byte("DSTV")) || len(b) < 10 || len(inputs)%8 == 0 || rng.Intn(8) == 0
		if !handshake {
			continue
		}
		r := bytes.NewReader(b)
		hello, err := wire.ReadHello(r)
		fmt.Printf("L %s %s %d %s %d\n", x(b), verdict(err), hello.MaxVersion, x(hello.Token), len(b)-r.Len())
		w, err := wire.ParseWelcome(b)
		fmt.Printf("W %s %s %d %d %d %d %d %s %s\n", x(b), verdict(err), w.Status, w.Version, w.MaxBody, w.Window, w.WindowBytes, verdict(w.Accept(1)), x(wire.AppendWelcome(nil, w)))
	}
}
