# Vectors written by the Go engine

Each file here is the Go engine's own answer to a list of inputs, and a Rust test replays it line for line. None is
edited by hand. `regenerate.sh` writes them all again.

| File | Written by | Rows | Checks |
| --- | --- | ---: | --- |
| `gojson-strings.txt` | `gojson_vectors.go`, **Go 1.27.0** | 2,075 | `gojson::write_string`, with and without HTML escaping |
| `gojson-reader.txt` | `gojson_reader_vectors.go`, **Go 1.27.0** | 1,595 | `gojson::parse`: what Go refuses, accepts and decodes |
| `seam-go.txt` | `seam_vectors.go` | 64 | the fixed clock and random source against `internal/seam` |
| `wire-go.txt` | `wire_vectors.go` | 7,998 | every verdict and decoded field of package `wire` on damaged frames |
| `crc32c-go.txt` | `internal_vectors_test.go.txt` | 300 | CRC-32C, whole and continued |
| `frame3-go.txt` | the same | 4,018 | frames, records, commits and topic definitions: bytes written, verdicts, fields, error texts |
| `checkpoint3-go.txt` | the same | 535 | `readCheckpoint3` on whole, cut, damaged and reordered checkpoints |
| `gotime-go.txt` | the same | 5,318 | what `Time.UnmarshalJSON` accepts, and the instant it reads |
| `base64-go.txt` | the same | 2,132 | what `encoding/base64` accepts and decodes |
| `recordscan-go.txt` | the same | 2,015 | the legacy journal's record scanner |
| `sha256-lengths.txt` | Python's `hashlib` | 200 | SHA-256 at lengths around the block boundaries |

The three `.go` programs carry `//go:build ignore`, so the Go module does not build them. `internal_vectors_test.go.txt`
calls unexported functions, so it is kept as text: `regenerate.sh` copies it into the package as a test file for one
run and removes it again.

The two JSON files define a format as Go 1.27.0 writes it, and `regenerate.sh` leaves them alone under any other
release. The other files were written with Go 1.24.7; they do not depend on the JSON encoder.
