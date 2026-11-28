# Row 1 receipts

| File | What it shows |
| --- | --- |
| `environment.txt` | machine, OS, toolchains, the commit of `main` underneath |
| `rust-check.txt` | `cargo fmt --check` and `cargo clippy -D warnings` exit 0; 26 tests pass, each named |

The wire codec is not reachable through the conformance driver, so no case and no trace exercises it yet. Its gate is the
line-for-line comparison with package `wire` in `rust-check.txt` (`every_decision_matches_the_go_codec`), beside the Go
package's own tests on the same vector file.
