# Row 2 receipts

| File | What it shows |
| --- | --- |
| `environment.txt` | machine, OS, toolchains, the commit of `main` underneath |
| `rust-check.txt` | `cargo fmt --check` and `cargo clippy -D warnings` exit 0; 38 tests pass, each named |

Frames, checkpoints and the record scanner are not reachable through the conformance driver until the store opens (row
5). Their gate is the line-for-line comparison with the Go functions in `rust-check.txt` (the `go_vectors` tests).
