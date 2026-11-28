//go:build ignore

// Writes more vectors that check the Rust wire codec against package wire: large inputs and the encoders.
//
//	go run rust/testdata/wire_more_vectors.go > rust/testdata/wire-more-go.txt
//
// Run from the repository root. wire_vectors.go damages small frames byte by byte. This program covers what that
// cannot: frames with fields at and beyond every length limit, up to a body of MaxBody and one byte more, where the
// order of the checks and the bounds show; and the three encoders, with what they refuse.
//
// Bytes are written compactly. An input is a list of runs, "hex*count" joined by commas ("-" for none), so a megabyte
// of one byte takes a few characters. A result of up to 48 bytes is written in hex between dashes, a longer one as
// its length and CRC-32C.
//
//	BH input verdict                                  the header is refused
//	BQ input verdict type id topic partition offset wait group key id value          DecodeRequest
//	BP input verdict type id partition offset found time key id value code           DecodeReply
//	BL input verdict maxversion token consumed                                       ReadHello
//	EQ type id topic partition offset wait group key id value verdict frame          AppendRequest
//	EP type id partition offset found time key id value code verdict frame           AppendReply
//	EH maxversion token verdict frame                                                AppendHello
//
// Every encoder is given a destination that already holds three bytes. "frame" is what it appended; after a refusal
// the destination must be as it was, and the row says "kept" or "changed".
package main

import (
	"bytes"
	"encoding/binary"
	"encoding/hex"
	"errors"
	"fmt"
	"hash/crc32"
	"io"
	"os"
	"runtime"
	"strings"

	"github.com/rodriveracom/dostoevsky/wire"
)

var castagnoli = crc32.MakeTable(crc32.Castagnoli)

type run struct {
	b []byte
	n int
}

type runs []run

func (r runs) bytes() []byte {
	var out []byte
	for _, p := range r {
		out = append(out, bytes.Repeat(p.b, p.n)...)
	}
	return out
}

func (r runs) String() string {
	var parts []string
	for _, p := range r {
		if len(p.b) > 0 && p.n > 0 {
			parts = append(parts, fmt.Sprintf("%s*%d", hex.EncodeToString(p.b), p.n))
		}
	}
	if len(parts) == 0 {
		return "-"
	}
	return strings.Join(parts, ",")
}

func of(b byte, n int) runs       { return runs{{[]byte{b}, n}} }
func lit(b ...byte) runs          { return runs{{b, 1}} }
func text(s string) runs          { return runs{{[]byte(s), 1}} }
func join(parts ...runs) (r runs) { return append(r, slicesConcat(parts)...) }

func slicesConcat(parts []runs) (r runs) {
	for _, p := range parts {
		r = append(r, p...)
	}
	return r
}

func show(b []byte) string {
	if len(b) <= 48 {
		return "-" + hex.EncodeToString(b) + "-"
	}
	return fmt.Sprintf("%d:%08x", len(b), crc32.Checksum(b, castagnoli))
}

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

func u16(v int) runs { return lit(byte(v), byte(v>>8)) }
func u64(v uint64) runs {
	return lit(binary.LittleEndian.AppendUint64(nil, v)...)
}

var out = os.Stdout
var rows = map[string]int{}

func row(format string, a ...any) {
	line := fmt.Sprintf(format, a...)
	rows[line[:2]]++
	fmt.Fprintln(out, line)
}

// frame writes the rows for one frame: a header whose length is the body's plus adjust, then the body.
func frame(t wire.Type, flags byte, id uint32, adjust int, body runs) {
	n := len(body.bytes()) + adjust
	head := lit(byte(n), byte(n>>8), byte(n>>16), byte(n>>24), byte(t), flags, byte(id), byte(id>>8), byte(id>>16), byte(id>>24))
	decode(join(head, body))
}

var seen = map[string]bool{}

func decode(in runs) {
	if seen[in.String()] {
		return
	}
	seen[in.String()] = true
	b := in.bytes()
	h, err := wire.ParseHeader(b)
	if err != nil {
		row("BH %s %s", in, verdict(err))
		return
	}
	q, err := wire.DecodeRequest(h, b[wire.HeaderSize:])
	row("BQ %s %s %d %d %s %d %d %d %s %s %s %s", in, verdict(err), q.Type, q.RequestID, show([]byte(q.Topic)), q.Partition, q.Offset, q.WaitMS,
		show([]byte(q.Group)), show([]byte(q.Key)), show([]byte(q.MessageID)), show(q.Value))
	p, err := wire.DecodeReply(h, b[wire.HeaderSize:])
	row("BP %s %s %d %d %d %d %t %d %s %s %s %d", in, verdict(err), p.Type, p.RequestID, p.Partition, p.Offset, p.Found, p.Time,
		show([]byte(p.Key)), show([]byte(p.MessageID)), show(p.Value), p.Code)
}

func kept(dst []byte) string {
	if string(dst) == "abc" {
		return "kept"
	}
	return "changed"
}

func main() {
	fmt.Fprintf(out, "# written by %s with rust/testdata/wire_more_vectors.go; do not edit\n", runtime.Version())
	keys := []runs{nil, of('k', wire.MaxKey), of('k', wire.MaxKey+1), of('k', 65535), of(0xff, 1), join(of('k', wire.MaxKey), of(0xff, 1))}
	ids := []runs{nil, of('i', wire.MaxID), of('i', wire.MaxID+1), of('i', 65535), of(0xc0, 1)}
	topics := []runs{text("t"), of('a', wire.MaxName), of('a', wire.MaxName+1), text("a b"), nil, of('a', 255)}
	str8 := func(r runs) runs { return join(lit(byte(len(r.bytes()))), r) }
	str16 := func(r runs) runs { return join(u16(len(r.bytes())), r) }

	// PUBLISH: every pairing of key and ID lengths, with small values and with values around the limits.
	for _, topic := range topics {
		for _, key := range keys {
			for _, id := range ids {
				fixed := len(str8(topic).bytes()) + len(str16(key).bytes()) + len(str16(id).bytes())
				sizes := []int{0, 300}
				// Megabyte values go with a few of the pairings: enough to show the order of the checks.
				if string(topic.bytes()) == "t" && len(key.bytes()) != 65535 && len(id.bytes()) != 65535 && (len(key.bytes()) != wire.MaxKey || len(id.bytes()) == 0) {
					sizes = append(sizes, wire.MaxValue-1, wire.MaxValue, wire.MaxValue+1, wire.MaxBody-fixed-1, wire.MaxBody-fixed, wire.MaxBody-fixed+1)
				}
				for _, size := range sizes {
					if size < 0 {
						continue
					}
					body := join(str8(topic), str16(key), str16(id), of('v', size))
					frame(wire.Publish, 0, 7, 0, body)
					if size == 300 || size >= wire.MaxValue-1 && key == nil && id == nil {
						frame(wire.Publish, 0, 7, 1, body)
						frame(wire.Publish, 0, 7, -1, body)
						frame(wire.Publish, 1, 7, 0, body)
						frame(wire.Publish, 0, 0, 0, body)
					}
				}
			}
		}
	}
	// A string field that announces more than the body holds, in a large body.
	frame(wire.Publish, 0, 7, 0, join(str8(text("t")), u16(65535), of('k', 65534)))
	frame(wire.Publish, 0, 7, 0, join(str8(text("t")), u16(4096), of('k', 4096), u16(65535), of('i', 300)))
	frame(wire.Publish, 0, 7, 0, join(lit(255), of('a', 254)))
	// Headers that announce a body at and beyond the maximum, with and without the body.
	for _, n := range []int{wire.MaxBody - 1, wire.MaxBody, wire.MaxBody + 1, 1 << 24, 1<<32 - 1} {
		head := lit(byte(n), byte(n>>8), byte(n>>16), byte(n>>24), byte(wire.Publish), 0, 7, 0, 0, 0)
		decode(head)
		decode(join(head, of(0, 300)))
		if n <= wire.MaxBody+1 {
			decode(join(head, of(0, n)))
			decode(join(head, lit(1, 't', 0, 0, 0, 0), of('v', n-6)))
		}
	}
	// COMMIT and POSITION: the group, the offset and the state, each at its limit, in every pairing.
	for _, group := range []runs{text("g"), of('g', wire.MaxName), of('g', wire.MaxName+1), text("g!"), nil} {
		for _, offset := range []uint64{0, 1<<63 - 2, 1<<63 - 1, 1 << 63, 1<<64 - 1} {
			for _, size := range []int{0, 300, wire.MaxValue, wire.MaxValue + 1} {
				for _, topic := range []runs{text("t"), text("a b")} {
					if size >= wire.MaxValue && (string(topic.bytes()) != "t" || len(group.bytes()) > 2 || offset == 1<<63-2 || offset == 1<<64-1) {
						continue
					}
					frame(wire.Commit, 0, 9, 0, join(str8(topic), u16(3), str8(group), u64(offset), of('s', size)))
				}
			}
		}
		frame(wire.Position, 0, 9, 0, join(str8(text("t")), u16(3), str8(group)))
		frame(wire.Position, 0, 9, 0, join(str8(text("t")), u16(3), str8(group), of(0, 300)))
	}
	for _, offset := range []uint64{0, 1<<63 - 1, 1 << 63} {
		for _, wait := range []int{0, wire.MaxWait, wire.MaxWait + 1, 65535} {
			frame(wire.Read, 0, 9, 0, join(str8(of('a', wire.MaxName)), u16(65535), u64(offset), u16(wait)))
			frame(wire.Read, 0, 9, 0, join(str8(of('a', wire.MaxName)), u16(65535), u64(offset), u16(wait), of(0, 300)))
		}
	}
	frame(wire.Ping, 0, 9, 0, of(0, 300))
	// Replies with large fields.
	for _, key := range keys {
		for _, id := range ids {
			for _, size := range []int{0, 300, wire.MaxValue, wire.MaxValue + 1} {
				for _, offset := range []uint64{5, 1 << 63} {
					if size >= wire.MaxValue && (offset != 5 || len(key.bytes()) > wire.MaxKey+1 || len(key.bytes()) == wire.MaxKey || len(id.bytes()) > wire.MaxID+1 || len(id.bytes()) == wire.MaxID) {
						continue
					}
					frame(wire.ReadOK, 0, 11, 0, join(lit(1), u16(2), u64(offset), u64(1790000000000000000), str16(key), str16(id), of('v', size)))
				}
			}
		}
	}
	for _, size := range []int{0, 300, wire.MaxValue, wire.MaxValue + 1, wire.MaxBody - 8, wire.MaxBody - 7} {
		frame(wire.PositionOK, 0, 11, 0, join(u64(5), of('s', size)))
		frame(wire.PositionOK, 0, 11, 0, join(u64(1<<63), of('s', size)))
		frame(wire.PublishOK, 0, 11, 0, join(u16(1), u64(5), of(0, size)))
		frame(wire.ReadOK, 0, 11, 0, join(lit(0), of(0, size)))
		frame(wire.ReadOK, 0, 11, 0, join(lit(2), of(0, size)))
		frame(wire.Error, 0, 0, 0, join(lit(3), of(0, size)))
		frame(wire.CommitOK, 0, 11, 0, of(0, size))
	}

	// HELLO: tokens at and beyond the limit, whole, cut short and followed by more.
	for _, n := range []int{0, 1, wire.MinToken - 1, wire.MinToken, 300, wire.MaxToken, wire.MaxToken + 1, 65535} {
		for _, have := range []int{n, n - 1, n + 300, 0} {
			if have < 0 {
				continue
			}
			for _, reserved := range []int{0, 1} {
				in := join(text("DSTV"), u16(1), u16(reserved), u16(n), of('s', have))
				if seen["L"+in.String()] {
					continue
				}
				seen["L"+in.String()] = true
				b := in.bytes()
				r := bytes.NewReader(b)
				hello, err := wire.ReadHello(r)
				row("BL %s %s %d %s %d", in, verdict(err), hello.MaxVersion, show(hello.Token), len(b)-r.Len())
			}
		}
	}

	// AppendRequest.
	types := []wire.Type{0, wire.Publish, wire.Read, wire.Position, wire.Commit, wire.Ping, 6, wire.PublishOK, wire.Error}
	groups := []runs{text("g"), of('g', wire.MaxName), of('g', wire.MaxName+1), text("g!"), nil}
	request := func(q wire.Request, topic, group, key, id, value runs) {
		q.Topic, q.Group, q.Key, q.MessageID, q.Value = string(topic.bytes()), string(group.bytes()), string(key.bytes()), string(id.bytes()), value.bytes()
		dst, err := wire.AppendRequest([]byte("abc"), q)
		tail := kept(dst)
		if err == nil {
			tail = show(dst[3:])
		}
		row("EQ %d %d %s %d %d %d %s %s %s %s %s %s", q.Type, q.RequestID, topic, q.Partition, q.Offset, q.WaitMS, group, key, id, value, verdict(err), tail)
	}
	for _, t := range types {
		for _, id := range []uint32{0, 1, 1<<32 - 1} {
			for _, topic := range topics {
				switch t {
				case wire.Publish:
					for _, key := range keys {
						for _, mid := range ids {
							for _, size := range []int{0, 300, wire.MaxValue, wire.MaxValue + 1} {
								if size >= wire.MaxValue && (id != 1 || string(topic.bytes()) != "t" && len(topic.bytes()) != wire.MaxName+1 || len(key.bytes()) == 65535 || len(mid.bytes()) == 65535 || len(mid.bytes()) == wire.MaxID) {
									continue
								}
								request(wire.Request{Type: t, RequestID: id, Partition: 9, Offset: 1 << 63, WaitMS: 65535}, topic, text("g!"), key, mid, of('v', size))
							}
						}
					}
				case wire.Read:
					for _, offset := range []uint64{0, 1<<63 - 1, 1 << 63, 1<<64 - 1} {
						for _, wait := range []uint16{0, wire.MaxWait, wire.MaxWait + 1, 65535} {
							request(wire.Request{Type: t, RequestID: id, Partition: 65535, Offset: offset, WaitMS: wait}, topic, nil, of(0xff, 5000), nil, nil)
						}
					}
				case wire.Position, wire.Commit:
					for _, group := range groups {
						for _, offset := range []uint64{0, 1<<63 - 2, 1<<63 - 1, 1 << 63} {
							for _, size := range []int{0, 300, wire.MaxValue, wire.MaxValue + 1} {
								if size >= wire.MaxValue && (id != 1 || t == wire.Position && offset != 0 || string(topic.bytes()) != "t" || len(group.bytes()) > 2 || offset == 1<<63-2) {
									continue
								}
								request(wire.Request{Type: t, RequestID: id, Partition: 3, Offset: offset}, topic, group, nil, nil, of('s', size))
							}
						}
					}
				default:
					request(wire.Request{Type: t, RequestID: id, Partition: 3, Offset: 1 << 63, WaitMS: 65535}, topic, text("g!"), of(0xff, 1), of(0xff, 1), of('v', 300))
				}
			}
		}
	}

	// AppendReply.
	reply := func(p wire.Reply, key, id, value runs) {
		p.Key, p.MessageID, p.Value = string(key.bytes()), string(id.bytes()), value.bytes()
		dst, err := wire.AppendReply([]byte("abc"), p)
		tail := kept(dst)
		if err == nil {
			tail = show(dst[3:])
		}
		row("EP %d %d %d %d %t %d %s %s %s %d %s %s", p.Type, p.RequestID, p.Partition, p.Offset, p.Found, p.Time, key, id, value, p.Code, verdict(err), tail)
	}
	for _, t := range []wire.Type{0, wire.Publish, wire.PublishOK, wire.ReadOK, wire.PositionOK, wire.CommitOK, wire.PingOK, 0x86, 0xfe, wire.Error} {
		for _, id := range []uint32{0, 1} {
			for _, offset := range []uint64{0, 1<<63 - 1, 1 << 63} {
				for _, code := range []wire.Code{0, 1, 3, 11, 12, 255} {
					for _, found := range []bool{false, true} {
						reply(wire.Reply{Type: t, RequestID: id, Partition: 2, Offset: offset, Found: found, Time: -5, Code: code}, text("k"), text("i"), text("v"))
					}
				}
				for _, key := range keys {
					for _, mid := range ids {
						for _, size := range []int{0, wire.MaxValue, wire.MaxValue + 1} {
							if size >= wire.MaxValue && (id != 1 || offset != 0 || t != wire.ReadOK && t != wire.PositionOK && t != wire.Error || len(key.bytes()) > wire.MaxKey+1 || len(key.bytes()) == 1 || len(mid.bytes()) != 0 && len(mid.bytes()) != wire.MaxID+1) {
								continue
							}
							reply(wire.Reply{Type: t, RequestID: id, Partition: 2, Offset: offset, Found: true, Time: 1790000000000000000, Code: 3}, key, mid, of('v', size))
						}
					}
				}
			}
		}
	}

	// AppendHello.
	for _, version := range []uint16{0, 1, 65535} {
		for _, n := range []int{0, 1, wire.MinToken, wire.MaxToken - 1, wire.MaxToken, wire.MaxToken + 1, 65535, 65536, 70000} {
			token := of('s', n)
			dst, err := wire.AppendHello([]byte("abc"), wire.Hello{MaxVersion: version, Token: token.bytes()})
			tail := kept(dst)
			if err == nil {
				tail = show(dst[3:])
			}
			row("EH %d %s %s %s", version, token, verdict(err), tail)
		}
	}
	fmt.Fprintf(os.Stderr, "wire-more-go.txt: %v\n", rows)
}
