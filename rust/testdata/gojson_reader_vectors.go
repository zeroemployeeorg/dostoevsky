//go:build ignore

// Writes the vectors that check the Rust JSON reader against encoding/json:
//
//	go run rust/testdata/gojson_reader_vectors.go > rust/testdata/gojson-reader.txt
//
// One case per line: the input in hex, then "invalid" where json.Valid refuses it, "valid" where it accepts a value
// that is not a string, or the decoded string in hex.
package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/rand"
	"runtime"
	"strings"
)

func main() {
	inputs := []string{
		`0`, `-0`, `01`, `1.`, `1.0`, `1e5`, `1E+5`, `1e`, `-`, `+1`, `.5`, `0x10`, `1e-`, `12 `, ` 12`, `1 2`, `NaN`, `Infinity`,
		`true`, `false`, `null`, `tru`, `nul`, `True`, `nulll`,
		`[]`, `[1]`, `[1,]`, `[,1]`, `[1 2]`, `[`, `]`, `[[]]`, `{}`, `{"a":1}`, `{"a":1,}`, `{a:1}`, `{"a"}`, `{"a":}`, `{"a":1,"a":2}`, `{"a":1 "b":2}`, `{1:2}`,
		"\t[\r\n1 ,\n2 ]\n", "\x0b1", " 1", "1\x00", `{"a":[{"b":null}]}`, ``, ` `, `"`, `"a`, `'a'`, `"a"b`,
		`"\"\\\/\b\f\n\r\t"`, `"\a"`, `"\x41"`, `"\u"`, `"\u12"`, `"\u12g4"`, `"\U0041"`, `"A"`, `"é"`, `"😀"`, `"😀"`,
		`"\ud83d"`, `"\ude00"`, `"\ud83dx"`, `"\ud83dA"`, `"\ud83d😀"`, `"\ude00\ud83d"`, `"\ud83d\\ude00"`, `"\ud83d\n"`, `"\udfff\udfff"`, `"�"`, `"\u0000"`,
		"\"\t\"", "\"\n\"", "\"\x00\"", "\"\x1f\"", "\"\x7f\"", "\"\xff\"", "\"a\xffb\"", "\"\xe2\x82\"", "\"\xe2\x82\xac\"", "\"\xed\xa0\x80\"", "\"\xc0\x80\"", "\"\xf0\x9f\x98\"",
		"\"\xf4\x90\x80\x80\"", "\"\xef\xbf\xbd\"", "\"  \"", "\"\xff\\ufffd\xfe\"",
		strings.Repeat("[", 10000) + strings.Repeat("]", 10000), strings.Repeat("[", 10001) + strings.Repeat("]", 10001),
		strings.Repeat(`{"a":`, 10000) + "1" + strings.Repeat("}", 10000), strings.Repeat(`{"a":`, 10001) + "1" + strings.Repeat("}", 10001),
	}
	rng := rand.New(rand.NewSource(20261128))
	pieces := []string{"a", `\"`, `\\`, `\/`, `\n`, `A`, `\ud83d`, `\ude00`, `é`, `\udc00`, `\ud800`, "\xff", "\xc2", "\xe2\x82", "é", "😀", "\xed\xa0\x80", `\`, "\x01", `\u12`, `\x`}
	for i := 0; i < 1500; i++ {
		var b strings.Builder
		b.WriteByte('"')
		for n := rng.Intn(8); n >= 0; n-- {
			b.WriteString(pieces[rng.Intn(len(pieces))])
		}
		b.WriteByte('"')
		inputs = append(inputs, b.String())
	}
	fmt.Printf("# written by %s with rust/testdata/gojson_reader_vectors.go; do not edit\n", runtime.Version())
	for _, in := range inputs {
		out := "invalid"
		if json.Valid([]byte(in)) {
			out = "valid"
			var s string
			if strings.HasPrefix(strings.TrimLeft(in, " \t\r\n"), `"`) && json.Unmarshal([]byte(in), &s) == nil {
				out = "-" + hex.EncodeToString([]byte(s)) + "-"
			}
		}
		fmt.Printf("-%s- %s\n", hex.EncodeToString([]byte(in)), out)
	}
}
