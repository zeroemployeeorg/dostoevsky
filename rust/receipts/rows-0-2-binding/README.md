# Receipts: request binding, reply bytes, the point rules, and the checks added to rows 1 and 2

Every path is relative to the repository root. Go here is 1.24.7; the differential files and
`rust/testdata/driver-requests-go.txt` are to be produced again with Go 1.27.0, which was not on this machine.

| File | What it shows |
| --- | --- |
| `environment.txt` | machine, toolchains, which Go driver was used |
| `rust-check.txt` | `make rust-check` with the pinned Rust 1.97.0: format, lints as errors, 51 tests |
| `go-verify.txt` | `make verify`: every Go package passes with the Rust workspace in the tree |
| `conformance.txt` | `make conformance-rust`: 28 pairings pass and 76 fail, as before. The 76 are the pairings that need the Rust store, which is not ported |
| `differential-selftest.txt` | the runner against stand-in drivers: 27 cases, each ending with the status it must |
| `differential-go-rust.txt`, `.json` | both drivers through every trace. All 22 streams of `request-binding` and the `seam-requests` trace are identical: same reply bytes, same exit status. The six traces that open a store differ at `open`, as they must before the store is ported |
| `differential-go-go.txt` | the Go driver against itself: 29 comparisons, none different, now compared as bytes |

## What each change is checked by

| Change | Check |
| --- | --- |
| A request is bound as Go binds it | `rust/crates/dostoevsky-cli/tests/go_vectors.rs` replays the Go driver's binding of 7,621 inputs: 5,683 bound, 1,935 refused, 3 empty. The `request-binding` trace sends 22 raw inputs to both drivers |
| A request is answered at its closing brace | `tests/driver_process.rs`, with a deadline; streams 20 and 21 of `request-binding`, for both drivers |
| Replies are compared as bytes | `scripts/differential_selftest.py`: `true` against `1`, `1` against `1.0`, member order, escaping, a repeated member and white space each count as a difference; a different message text does not |
| The runner's deadline, exit status and empty formats | the same self-test: a driver that hangs, one that stops, a format with no request, no requests, no formats, a request without an op, a trace that is not JSON, wrong arguments: status 2 each |
| The kill line | `rust/crates/dostoevsky-core/tests/kill_point.rs`: a process armed with skip 2 writes exactly `seam: killing at segment.sync, pass 3` and dies by SIGKILL |
| No second `arm` over an unchecked point | the unit test in `seam.rs`, for a point that has not fired and for one that has |
| `Due` and `Fire` as two steps | `seam::due` and `seam::fire`, in the same unit test |
| The error-order test can fail | it compares `Kind::ALL` with the `codes` table read from `cmd/dost-conform/main.go` |
| The base64 test can fail | 30 named rows; the test counts the rows with loose final bits and with line ends |
| Large wire inputs, encoder refusals | `rust/crates/dostoevsky-wire/tests/go_more_vectors.rs`: 8,545 rows, 372 inputs above a megabyte |
| Time to the frame's time field | `the_frame_time_field_matches_go`: 948 rows |
| `seal_frame3` checks its slice | `sealing_checks_that_a_header_is_there` |
| The record scanner with Go 1.27.0's rows | `recordscan-go.txt`, 2,018 rows, all reproduced |

## Not done

- **The `subtle` crate.** No package registry could be reached, so `digests_equal` in the wire crate is still a loop
  with no guarantee. `rust/DEPS.md` lists the crate. The wire codec is not complete until it is in.
- **Go 1.27.0.** The differential receipts and `driver-requests-go.txt` come from Go 1.24.7.
- **The two point rules against the Go driver.** The Go driver built here has neither, so the refusal's text, the
  order of its checks and the kill line are checked in Rust only.
- **A request marked as the one that dies.** No ported module reaches a point, so no trace has one yet, and the
  differential runner does not read a driver's standard error. That comes with the first module that reaches a point
  through the driver.
