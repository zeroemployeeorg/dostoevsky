# The Rust engine

Dostoevsky is being rebuilt in Rust, in two phases. Phase 1, here, is a faithful port of the Go engine in the
repository root: module by module, the same bytes on disk and on the wire, the same error codes. The Go engine keeps
shipping until the Rust engine passes every gate.

**State: rows 0 to 2 of 14.** The layout, the test drivers, the wire codec and the frame formats exist. Nothing opens
a store yet.

| Read | For |
| --- | --- |
| `PORT-NOTES.md` | the map from Go files to Rust modules, what Go does and what Rust does, what is not yet known |
| `DEPS.md` | the pinned toolchain, outside crates (none yet), own code where a crate would be usual |
| `differential/README.md` | the differential driver and its traces |
| `testdata/README.md` | the vectors written by the Go engine, and how to write them again |
| `receipts/` | the output of each row's gates |

## Crates

| Crate | Holds |
| --- | --- |
| `dostoevsky-wire` | the codec, no I/O |
| `dostoevsky-format` | frames, checkpoints, sidecars and the journal encoding, no I/O; JSON as Go writes and reads it |
| `dostoevsky-core` | the store; the test seam; the error kinds; `peers`; the `sketch` package |
| `dostoevsky-workflow` | the workflow layer |
| `dostoevsky-server` | the server; `clienthttp` and the HTTP gateway |
| `dostoevsky-cli` | the command-line tools and the conformance driver |
| `dostoevsky-py`, `dostoevsky-node` | the bindings |

## Gates

From the repository root:

```sh
make rust-check          # cargo fmt --check, cargo clippy -D warnings, cargo test
make conformance-rust    # every case, each engine opening the other's stores
make differential        # the same requests to both engines; the stores and replies must be equal
```

The toolchain is pinned in `rust-toolchain.toml`. Build output goes to `.dev/rust-target`.
