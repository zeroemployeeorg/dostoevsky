//go:build ignore

// Writes the string vectors that check the Rust JSON writer against encoding/json:
//
//	go run rust/testdata/gojson_vectors.go > rust/testdata/gojson-strings.txt
//
// One case per line: the input, json.Marshal's output, and the output with HTML escaping off, all in hex. Inputs that
// are not valid UTF-8 come last, under a header that records which form this toolchain writes for them.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/rand"
	"runtime"
	"strings"
	"unicode/utf8"
)

func line(s string) string {
	html, err := json.Marshal(s)
	if err != nil {
		panic(err)
	}
	var plain bytes.Buffer
	e := json.NewEncoder(&plain)
	e.SetEscapeHTML(false)
	if err := e.Encode(s); err != nil {
		panic(err)
	}
	return fmt.Sprintf("-%s- %s %s", hex.EncodeToString([]byte(s)), hex.EncodeToString(html), hex.EncodeToString(bytes.TrimSuffix(plain.Bytes(), []byte("\n"))))
}

func main() {
	var valid, invalid []string
	add := func(s string) {
		if utf8.ValidString(s) {
			valid = append(valid, s)
		} else {
			invalid = append(invalid, s)
		}
	}
	add("")
	for b := 0; b < 256; b++ {
		add(string([]byte{byte(b)}))
		add("a" + string([]byte{byte(b)}) + "z")
	}
	for _, r := range []rune{0x7f, 0x80, 0xa0, 0xe9, 0x7ff, 0x800, 0x2027, 0x2028, 0x2029, 0x202a, 0xd7ff, 0xe000, 0xfffd, 0xfffe, 0xffff, 0x10000, 0x1f600, 0x10ffff} {
		add(string(r))
		add("<" + string(r) + "&")
	}
	// Truncated, overlong and surrogate encodings, and continuation bytes on their own.
	for _, s := range []string{"\xc2", "\xe2\x82", "\xf0\x9f\x98", "\xc0\x80", "\xc1\xbf", "\xe0\x80\x80", "\xed\xa0\x80", "\xed\xbf\xbf", "\xf4\x90\x80\x80", "\xf8\x88\x80\x80\x80", "\x80\x80", "\xe2\x28\xa1", "ok\xffok\xfe"} {
		add(s)
		add("x" + s + "y" + s)
	}
	rng := rand.New(rand.NewSource(20261128))
	alphabet := []string{"a", "\"", "\\", "/", "<", ">", "&", "\n", "\t", "\x00", "\x1f", "\x7f", "é", "\u2028", "\u2029", "😀", "\xff", "\xc2", "\xe2\x82", "\xed\xa0\x80"}
	for i := 0; i < 1500; i++ {
		var b strings.Builder
		for n := rng.Intn(12); n >= 0; n-- {
			b.WriteString(alphabet[rng.Intn(len(alphabet))])
		}
		add(b.String())
	}
	fmt.Printf("# written by %s with rust/testdata/gojson_vectors.go; do not edit\n", runtime.Version())
	for _, s := range valid {
		fmt.Println(line(s))
	}
	form := "raw"
	if out, _ := json.Marshal("\xff"); string(out) == `"\ufffd"` {
		form = "escaped"
	}
	fmt.Printf("# invalid-utf8: %s\n", form)
	for _, s := range invalid {
		fmt.Println(line(s))
	}
}
