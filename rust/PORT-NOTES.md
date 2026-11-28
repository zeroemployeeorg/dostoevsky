# Port notes

Phase 1 of the Rust engine is a faithful port of the Go engine: the same on-disk bytes, the same wire bytes, the same
error codes, the same results on the conformance suite. This file records how the Rust code maps to the Go code, each
Go behavior the port keeps on purpose, each it changes, and what is not yet known.

## State

| Row | What | State |
| ---: | --- | --- |
| 0 | Workspace, layout, JSON as Go writes and reads it, the test seam, the conformance driver, the differential driver | Done |
| 1 | Wire codec | Done |
| 2 | Segment frames, checkpoints, the record scanner | Done, except the replayer, which is ported with the log state (note 16) |
| 3 to 14 | Everything that opens a store | Not started |

No engine request works yet. The Rust driver answers each one with the error `other` and names the row that brings it.

## Map from Go to Rust

One Rust module per Go file, with the same name, so the two can be read side by side. A module that is still a stub
says so in its first line and names its row.

| Row | Go | Crate | Module | State |
| ---: | --- | --- | --- | --- |
| 0 | `cmd/dost-conform/main.go`, `workflow.go` | `dostoevsky-cli` | `conform`, binary `dost-conform` | ported; engine calls answer "not ported" |
| 0 | `internal/seam/seam.go` | `dostoevsky-core` | `seam` | ported |
| 0 | `internal/seam/points.go` | `dostoevsky-core` | `seam` (`POINTS`, `arm`, `disarm`, `due`, `fire`, `at`) | ported; no ported module reaches a point yet |
| 0 | the sentinel errors in `node.go` and its neighbours | `dostoevsky-core` | `errors` | ported |
| 0 | `encoding/json` as the engine uses it | `dostoevsky-format` | `gojson`, `base64` | ported for strings, integers and bytes |
| 1 | `wire/wire.go` | `dostoevsky-wire` | `wire` | ported |
| 2 | `frame3.go` | `dostoevsky-format` | `frame3` | ported |
| 2 | `checkpoint3.go` (format half) | `dostoevsky-format` | `checkpoint3` | ported |
| 2 | `recordscan.go` | `dostoevsky-format` | `recordscan` | ported |
| 2 | `crcTable` in `journal.go` | `dostoevsky-format` | `crc32c` | ported |
| 2 | `Time.UnmarshalJSON` | `dostoevsky-format` | `gotime` | ported (reading) |
| 3 | `admission.go` | `dostoevsky-core` | `admission` | stub |
| 3 | `id_index.go` | `dostoevsky-core` | `id_index` | stub |
| 3 | `idfilter.go` | `dostoevsky-format` | `idfilter` | stub |
| 3 | `location_index.go` | `dostoevsky-core` | `location_index` | stub |
| 3 | `segindex.go` | `dostoevsky-format` | `segindex` | stub |
| 4 | `journal.go` | `dostoevsky-core` | `journal` | stub |
| 4 | `logstate.go` | `dostoevsky-core` | `logstate` | stub |
| 4 | `sync_linux.go, sync_other.go` | `dostoevsky-core` | `sync` | stub |
| 5 | `seglayer.go` | `dostoevsky-core` | `seglayer` | stub |
| 5 | `segmented_node.go` | `dostoevsky-core` | `segmented_node` | stub |
| 6 | `batch.go` | `dostoevsky-core` | `batch` | stub |
| 6 | `cap.go` | `dostoevsky-core` | `cap` | stub |
| 6 | `node.go` | `dostoevsky-core` | `node` | stub |
| 6 | `notify.go` | `dostoevsky-core` | `notify` | stub |
| 6 | `publish_batch.go` | `dostoevsky-core` | `publish_batch` | stub |
| 6 | `publish_queue.go` | `dostoevsky-core` | `publish_queue` | stub |
| 6 | `stream.go` | `dostoevsky-core` | `stream` | stub |
| 7 | `retention.go` | `dostoevsky-core` | `retention` | stub |
| 8 | `backup.go` | `dostoevsky-core` | `backup` | stub |
| 8 | `migration.go` | `dostoevsky-core` | `migration` | stub |
| 8 | `repair.go` | `dostoevsky-core` | `repair` | stub |
| 8 | `upgrade.go` | `dostoevsky-core` | `upgrade` | stub |
| 9 | `workflow_log.go` | `dostoevsky-core` | `workflow_log` | stub |
| 10 | `workflow/store.go` | `dostoevsky-workflow` | `store` | stub |
| 11 | `wireserver/conn.go` | `dostoevsky-server` | `conn` | stub |
| 11 | `wireserver/pool.go` | `dostoevsky-server` | `pool` | stub |
| 11 | `wireserver/sequencer.go` | `dostoevsky-server` | `sequencer` | stub |
| 11 | `wireserver/server.go` | `dostoevsky-server` | `server` | stub |
| 12 | `clienthttp/handler.go` | `dostoevsky-server` | `clienthttp` | stub |
| 12 | `peers.go` | `dostoevsky-core` | `peers` | stub |
| 13 | `sketch/hll.go` | `dostoevsky-core` | `sketch::hll` | stub |
| 13 | `sketch/hot_topics.go` | `dostoevsky-core` | `sketch::hot_topics` | stub |
| 13 | `sketch/latency.go` | `dostoevsky-core` | `sketch::latency` | stub |
| 4 | `checkpoint3.go` (the replayer) | `dostoevsky-core` | with `logstate` | not started |
| 8 | `cmd/dost-repair`, `cmd/dost-storage` | `dostoevsky-cli` | binaries of the same names | not started |
| 11 | `cmd/dostoevsky-server` | `dostoevsky-server` | binary of the same name | not started |
| 12 | `cmd/dostoevsky-gateway` | `dostoevsky-server` | binary of the same name | not started |
| 14 | (none) | `dostoevsky-py`, `dostoevsky-node` | | empty |

`peers` and the `sketch` package are in `dostoevsky-core`. `clienthttp` and the gateway binary are in
`dostoevsky-server`. Not ported in phase 1: `cmd/plots`, `cmd/sketchreport`, `examples/`, `benchmarks/`, the existing
clients. `benchmarks/rustcore` is a frozen record of an earlier prototype: it lies outside `rust/`, is not a workspace
member, and the gates do not touch it.

Where a Go file mixes encoding with file access, the encoding goes to `dostoevsky-format` and the file access to
`dostoevsky-core`. The placement of the row 3 and row 4 files in the table is provisional until they are ported.

## What Go does, and what Rust does

### The JSON encoding

1. **The format is the output of Go 1.27.0's `encoding/json`.** Go writes `<`, `>` and `&` as `\u003c`, `\u003e`,
   `\u0026`; U+2028 and U+2029 as escapes; control bytes as `\b`, `\f`, `\n`, `\r`, `\t` or `\u00XX`; struct fields in
   declaration order and map keys sorted by bytes. Rust (`gojson`) writes the same bytes. Checked against 2,075 strings
   written by Go 1.27.0.
2. **Invalid UTF-8.** Go 1.27.0 writes each invalid byte as the three bytes EF BF BD; earlier releases wrote the six
   characters `\ufffd`. Rust writes the 1.27.0 form and reads both. Go replaces one invalid byte at a time; Rust's own
   lossy conversion replaces a whole invalid sequence at once, so `gojson` does not use it.
3. **What Go's decoder accepts.** Invalid UTF-8 inside strings, unpaired surrogate escapes (replaced by U+FFFD, and what
   follows read again on its own), duplicate keys (the last wins), and nesting up to 10,000. Rust accepts the same and
   refuses the same. Checked against 1,595 inputs through Go 1.27.0's decoder.
4. **Nesting to 10,000 must not overflow the stack.** Go's stacks grow; a Rust thread's does not. The reader, the
   writer and the destructor of `gojson::Value` keep their own stacks.
5. **A nil byte slice and an empty one differ on the wire.** Go writes a nil `[]byte` as `null` and an empty one as
   `""`. Rust carries such fields as `Option<Vec<u8>>`, and each ported function returns the same one of the two as Go.
   Checkpoints turn an empty group state into nil (`cloneState`); `CpGroup3::state` is `None` for empty.
6. **Times.** Go's `Time.UnmarshalJSON` accepts more than RFC 3339: an hour of one digit, a comma before the fraction
   of a second, a zone offset of up to 24 hours and 60 minutes. The stricter checks in the Go source are switched off.
   Rust (`gotime`) accepts exactly the same inputs. Checked against 5,318 inputs. Writing times, and floating-point
   numbers, are added in the row that first needs them.

### The driver

7. **Requests are read as Go's decoder reads the request struct.** A key matches a field by exact name, else without
   regard to case. `null` leaves a field at its zero value. An unknown field, a value of the wrong type, or an integer
   written with a fraction or exponent ends the driver with status 1 and no reply. A request may span lines.
8. **A Go error can wrap several sentinels,** and the Go driver reports the first match in a fixed order. Rust's
   `errors::Error` carries every kind it wraps and `Error::code` applies that order. The three workflow codes
   (`start_conflict`, `stale`, `missing`) join with the workflow layer.
9. **`fix_inputs` must come before anything is opened.** Go's `seam.Fix` is not safe while a node or store is in use.
   The Rust driver refuses `fix_inputs` once a node or store is open.

### The seam

10. **Clock and random source follow Go's `internal/seam`.** The k-th reading after `fix` (k from 0) is
    `start + k * step`; `advance` moves the clock without counting as a reading; random bytes are
    SHA-256(seed || counter) blocks with seed and counter as 8 bytes little-endian, and what is left of a request's last
    block is not used again. Checked against 20 readings and 32 requests made of the Go package.
11. **Write order.** Where Go writes partitions and consumer groups it holds in maps (legacy retention, the legacy
    export of a segmented store, migration), it writes partitions ordered by topic and then number, and groups by name.
    Checkpoints order partitions by their stored identifier. Rust must write in these orders; a Go map written in map
    order anywhere else is a defect to report, not a behavior to copy. Nothing ported so far writes from a map.
12. **Named points.** Go's `internal/seam/points.go` lists 18 points. `Arm` refuses a name that is not in the list. An
    armed point fires exactly once, the first time it is reached after `skip` earlier times; `skip` is signed, and with
    a negative value the point never fires. `Disarm` reports whether the point fired. A point that does work before it
    fires calls `Due`, then `Fire`; every other point calls `At`. Rust's `seam` carries the same list and the same
    functions, and the driver's `arm` and `disarm` reply as Go's do. Each ported module must reach each point at the
    boundary its Go counterpart does; nothing ported so far contains one.
13. **The system's random source.** Unset, Go reads `crypto/rand`; Rust reads `/dev/urandom`. Platforms without that
    file need another source before the workflow layer can run there.

### The wire codec

14. **Names, keys and message IDs are bytes.** They are Go strings, checked for UTF-8 only where the protocol says so.
    Rust uses byte slices for the same reason. A decoder returns what it decoded together with its verdict, as the Go
    functions do: a refused request still carries the ID its ERROR must answer.
15. **`io.ReadFull` tells an input that ended before a part of the HELLO from one that ended inside it.** Rust's
    `read_exact` does not, so `read_hello` reads in its own loop and reports `Eof` or `UnexpectedEof` as Go would.

### Frames and checkpoints

16. **`checkpoint3.go` is split.** Its encoding (frames, the builder, `readCheckpoint3`) is in `dostoevsky-format`. Its
    replayer applies frames to the log state, so it is ported with `logstate`, in `dostoevsky-core`, in row 4.
17. **Record times.** Go carries a `time.Time` and converts at the frame, refusing an instant outside what an i64 of
    nanoseconds holds. Rust carries the nanoseconds, so such a time cannot be formed and there is nothing to refuse.
    `i64::MIN` stands for Go's zero time, as in the frame.
18. **The scanner's integer reader wraps on overflow.** Go's `canonicalUint` multiplies before it checks the digit
    count, and Go integers wrap. Rust uses wrapping arithmetic there; ordinary arithmetic panicked on a 19-digit number.
19. **Error texts.** Only error codes are compared between engines. The frame and checkpoint errors nevertheless carry
    Go's texts word for word, and the vectors check them.

### The build

20. **The source release copies `rust/` as it stands** (`scripts/release.sh`; a test requires every top-level directory
    to be listed). Cargo's build output therefore goes to `.dev/rust-target`, outside `rust/`.
21. **Fuzz targets.** `wire_test.go` and `frame3_test.go` have Go fuzz targets. Rust has none yet. In their place the
    tests replay the Go functions' own results on inputs damaged in many small ways (7,998 wire frames; 4,018 frame
    rows; 535 checkpoint streams).

## Observed in the Go engine

22. **Without the seam, two runs of the Go engine differ in more than record times.** The differential driver, run
    with the Go engine against itself, shows differences in the legacy journal and the segment logs, and also in the
    `.idx` and `.bloom` sidecars and in a backup's `journal.v1`. **With the seam fixed they all vanish:** with a Go
    driver that serves `fix_inputs`, the Go engine against itself is identical in every trace, replies and files, in
    six runs out of six (`receipts/seam/`).
23. **A case without a `switch` step never runs its second engine.** The runner counts a case once per ordered pair of
    engines, and uses the second only after a `switch`. `incomplete-backup-directory` has none, so its pairing "Go then
    Rust" is the Go engine alone and passes whatever the Rust driver does. Two of the 28 passes of a run with both
    drivers are of this kind (`receipts/seam/conformance-pairings.txt`).

## Not yet known

- **Go 1.27.0** was not on the machine that wrote the vector files other than the two JSON files. `seam-go.txt`,
  `wire-go.txt` and the files written by `internal_vectors_test.go.txt` come
  from Go 1.24.7. Nothing in them depends on the JSON encoder's change between those releases, but they should be
  written again with 1.27.0 (`rust/testdata/regenerate.sh`).
- **The lint gate on releases newer than the pinned one.** `sha256.rs` and `crc32c.rs` avoid `chunks_exact`, which
  newer lints object to; that could not be confirmed with a newer release.
- **Wire bytes of a server.** The differential driver compares stores and driver replies. Comparing wire bytes needs
  the server (row 11).
- **Other platforms.** Everything ran on one x86-64 Linux machine.
