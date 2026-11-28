# The differential test

`scripts/differential.py` sends one trace of requests to two or more engines and compares what they wrote, byte for
byte. It is the check behind "the same on-disk bytes" in phase 1.

```sh
make differential
python3 scripts/differential.py --engine go=GO_DRIVER --engine rust=RUST_DRIVER [--trace NAME] [--format legacy]
python3 scripts/differential.py --engine go=GO_DRIVER --engine again=GO_DRIVER --without-seam   # an engine against itself
```

Linux, Python 3.10 or later, standard library only. The engines are driven through their conformance drivers
(`cmd/dost-conform/README.md`), so nothing is translated twice.

## How it works

- **One request at a time.** Group commit under concurrency depends on timing. Concurrent behavior belongs to the
  conformance suite, which compares state, not bytes.
- **The same request bytes for every engine.** Each engine runs a trace in its own new directory, which is also its
  working directory, and a trace names its stores by relative paths (`store`, `copy`). An absolute path is refused.
- **Fixed inputs.** Before the trace, the runner sends `fix_inputs`, which sets the engine's clock and random source
  (the test seam). `--without-seam` leaves that out, for a driver that has no seam yet; then even one engine run twice
  differs, in every record time at least.
- **Refusals that may differ in code.** A request marked `"refuses": true` in a trace is one where engines may report
  different error codes: retention given several invalid floors may report any one of them. For it the runner requires
  only that every engine refuses and that the refusal leaves the engine's own directory unchanged.
- **What is compared,** against the first engine named:
  - the bytes of each reply line. One thing is masked first: the text of the top-level `message` member, when it is a
    string, because the suite compares error codes and never their text. Nothing is parsed into numbers, booleans or
    maps, so `true` against `1`, the order of members, the escaping of a string and a member given twice all count as
    differences. The number of lines identical without the mask is reported beside it;
  - the two directory trees: the set of paths, each file's bytes, each symbolic link's target;
  - for a stream (below), the exit status of each driver.
- **A deadline on every read.** A driver has `--deadline` seconds (120 unless given) for each reply and for its exit.
  One that does not answer in time is killed and the comparison is reported as broken.
- **Exit status:** 0 identical, 1 different, 2 anything else: a driver that could not run a trace, a trace that is not
  well formed, a format a trace names but has no request for, a missing trace, wrong arguments.
- `--receipt FILE` writes the counts of every comparison as JSON. `--keep DIR` keeps the directories for inspection.
  `--traces DIR` reads traces from another directory.

`scripts/differential_selftest.py` runs the runner against stand-in drivers whose replies are known and checks the
status it ends with: 13 kinds of reply pair that must or must not count as a difference, a driver that hangs, one
that stops, and every malformed trace. `make differential` runs it first.

## Traces

`traces/*.json`, in one of two forms.

**Requests:** `about`, `formats` (`legacy`, `segmented`) and `requests`. A request is a driver request, with the same
conveniences as the conformance case files: `open` gets a default directory, format, node and topics; `value` and
`state` are written as text; `topic` defaults to `events`. `only` limits a request to one format; a format left with
no request is refused.

**Streams:** `about` and `streams`. Each stream has `about` and `input`: the exact text one new driver process
receives. Nothing is added to it, not even `fix_inputs`, so it can hold what a request written as JSON cannot: a key
given twice, a name in another case, a value with no newline after it. `replies_before_end` is the number of replies
the runner waits for while the driver's input is still open; a driver that waits for more input before it answers
runs into the deadline.

| Trace | Requests (legacy, segmented) | What it exercises |
| --- | ---: | --- |
| `publish-read-commit` | 24, 25 | every record field, a repeated ID, a batch across topics, reads, commits with state, a reopen |
| `retention` | 16, 16 | retention refused above a group's position, retention with two invalid floors, retention twice, an ID reused after retention, a reopen |
| `backup-and-migrate` | 16, 11 | a backup, a backup onto an existing directory, migration to the segmented format, inspection |
| `seam-requests` | 12 (one format) | the seam requests themselves, with nothing open: arming a point that exists and one that does not, a negative skip, what `disarm` replies. Each point is armed only after `disarm` reported the one before |
| `request-binding` | 22 streams | how a driver reads its input: bytes as numbers, folded names, `null` for a request and for elements, a key given twice (text, lists, bytes, maps, two cases), what ends the driver with status 1, and two requests answered with no newline while the input stays open |

Requests that must fail (an unknown topic, a stale commit, a read past the end) are part of the traces: the error codes
are compared too.

## Limits

- It compares stores and driver replies. Comparing wire bytes needs the server and is not built.
- The traces are a start, not coverage. Each row of the port adds traces for what it ports.
- A driver without `fix_inputs` stops at it; run such a driver with `--without-seam`.
