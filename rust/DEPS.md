# Dependencies

## Toolchain

`rust-toolchain.toml` pins **Rust 1.97.0** (`rustc 1.97.0 (2d8144b78 2026-07-07)`), with `rustfmt` and `clippy`. One
exact release makes the format and lint gates give the same answer everywhere. It was more than 14 days old when it was
pinned. A newer release is adopted by changing that file and this line together, after the lint gate passes with it.

`rustfmt.toml` sets the width to 120 columns, the width of the Go sources.

## Outside crates

Rule: every outside crate is listed here with its version, its registry link and the reason for it, and is at least 14
days old when it is pinned.

| Crate | Version | Released | License | Registry | Why |
| --- | --- | --- | --- | --- | --- |
| `subtle` | =2.6.1 | 2024-06-24 | BSD-3-Clause | <https://crates.io/crates/subtle/2.6.1> | the handshake's digest comparison in constant time, where Go uses `subtle.ConstantTimeCompare`. 2.6.0 is yanked and is not used |

Everything else builds from the standard library and the workspace's own crates.

## Expected, not yet added

No version has been chosen for any of these and no age has been checked. Each is added, with its version, link and
release date, when it is first needed.

| Crate | Needed for | First row |
| --- | --- | ---: |
| `sha2` | SHA-256 in the wire handshake and the test seam; replaces `dostoevsky-format::sha256` | now |
| `tokio` | the server's network I/O | 11 |
| `rustls` | TLS in the server and the gateway | 11 |
| `pyo3` | the Python binding | 14 |
| `napi`, `napi-derive` | the Node addon | 14 |
| `wasm-bindgen` | WebAssembly builds of the two crates without I/O | 14 |

## Own code where a crate would be usual

| Code | Where | Why |
| --- | --- | --- |
| JSON reader and writer | `dostoevsky-format::gojson` | The format is the output of Go's encoder, byte for byte, and what Go's decoder accepts |
| Reading times | `dostoevsky-format::gotime` | What Go's `Time.UnmarshalJSON` accepts is wider than RFC 3339 |
| base64 | `dostoevsky-format::base64` | Part of the same Go-compatible encoding |
| CRC-32C | `dostoevsky-format::crc32c` | Small, and uses the processor's instructions directly |
| SHA-256 | `dostoevsky-format::sha256` | **Temporary.** `sha2` replaces it as soon as a registry can be reached. Until then it is checked against the standard vectors and 200 digests from another implementation |
