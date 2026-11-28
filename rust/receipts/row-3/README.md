# Row 3 receipts

| File | What it shows |
| --- | --- |
| `environment.txt` | machine, OS, toolchains, the commit of `main` underneath |
| `rust-check.txt` | `cargo fmt --check` and `cargo clippy -D warnings` exit 0; 42 tests pass, each named |

The index structures are not reachable through the conformance driver until the store opens (row 5). Their gate is the
line-for-line comparison with the Go functions in `rust-check.txt`: `the_id_table_and_filter_match_go`,
`the_segment_index_matches_go` and `the_admission_sketch_and_the_location_index_match_go`.
