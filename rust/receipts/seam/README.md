# Receipts for the seam requests

| File | What it shows |
| --- | --- |
| `environment.txt` | machine, toolchains, which Go driver was used |
| `differential-go-go-with-seam.txt` | the Go engine against itself with the clock and randomness fixed: identical in all 7 comparisons, six runs out of six |
| `differential-go-rust.txt` | both drivers answer the seam requests with the same bytes (11 replies of 11); on the traces that open a store, Rust still differs at `open`, as it must before the store is ported |
| `conformance-pairings.txt` | `incomplete-backup-directory` with both drivers: the pairing "Go then Rust" passes because the case has no `switch` step, so only the first engine runs |
