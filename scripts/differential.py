#!/usr/bin/env python3
"""Runs one trace of requests through two or more engines and compares what they wrote, byte for byte.

    python3 scripts/differential.py --engine go=PATH/TO/dost-conform --engine rust=PATH [--trace NAME] [--format F]

Standard library only. The engines are driven through the conformance drivers (cmd/dost-conform/README.md), one
request at a time, because group commit under concurrency depends on timing and is the conformance suite's business.

Each engine runs every trace in its own new directory, which is also its working directory: a trace names its stores by
relative paths, so every engine receives the same request bytes. With the test seam (the `fix_inputs` request) the clock
and the random source are fixed first, and two engines that agree write identical trees and give identical replies.

A request marked "refuses" in a trace is one where engines may report different error codes (retention given several
invalid floors may report any one of them). For it the runner requires only that every engine refuses and that the
refusal leaves the engine's own directory unchanged.

A trace of "streams" instead of "requests" checks how a driver reads its input. Each stream is the exact text one new
driver process receives, so it can hold what a request written as JSON here cannot: a key given twice, a name in
another case, a value with no newline after it. Nothing is added to it, not even fix_inputs. The runner compares the
bytes each driver wrote and the status it exited with.

Compared, against the first engine named:
  replies   the bytes of each reply line. One thing is masked first: the text of the top-level "message" member, when
            it is a string, because the suite compares error codes and never their text. Nothing is parsed into
            numbers, booleans or maps, so true against 1, the order of members, the escaping of a string and a member
            given twice all count as differences. The number of lines identical without the mask is reported too.
  trees     the set of paths, each file's bytes, and each symbolic link's target
  status    for a stream, the exit status of each driver

Every read from a driver has a deadline (--deadline, 120 seconds unless given): a driver that does not answer in time is
killed and the comparison is reported as broken.

Exit status: 0 identical, 1 different, 2 anything else: a driver that could not run a trace, a trace that is not well
formed or has no requests for a format it names, a missing trace, wrong arguments. The trace format is in
rust/differential/README.md.
"""
import argparse
import base64
import glob
import hashlib
import json
import os
import select
import subprocess
import sys
import tempfile
import time

TRACES = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "rust", "differential", "traces")
DEADLINE = 120
FORMATS = ("legacy", "segmented")
INPUTS = {"start": 1790000000000000000, "step": 1000, "seed": 20261128}


class Broken(Exception):
    """A driver could not run the trace at all: not a difference between engines."""


def text(value):
    return base64.b64encode(value.encode()).decode()


def requests_of(trace, fmt):
    """The trace as the list of driver requests for one format, with the conveniences of the case files."""
    out, loose = [], set()
    for step in trace["requests"]:
        if step.get("only", fmt) != fmt:
            continue
        request = {k: v for k, v in step.items() if k not in ("only", "refuses")}
        if request["op"] == "open":
            request.setdefault("dir", "store")
            request.setdefault("segmented", fmt == "segmented")
            request.setdefault("node", "a")
            request.setdefault("nodes", ["a"])
            request.setdefault("topics", {"events": 1})
        if request["op"] in ("publish", "read", "read_batch", "position", "commit_batch"):
            request.setdefault("topic", "events")
        for key in ("value", "state"):
            if key in request:
                request[key] = text(request[key])
        if "items" in request:
            request["items"] = [dict(m, value=text(m.get("value", "")), topic=m.get("topic", "events")) for m in request["items"]]
        if "floors" in request:
            request["floors"] = [dict({"topic": "events", "partition": 0}, **f) for f in request["floors"]]
        for key in ("dir", "dest"):
            if key in request and os.path.isabs(str(request[key])):
                raise Broken("a trace must name stores by relative paths, not %s" % request[key])
        out.append(request)
        if step.get("refuses"):
            loose.add(len(out) - 1)
    return out, loose


class Driver:
    """One driver process. Every read has a deadline."""

    def __init__(self, name, binary, root, deadline):
        self.name, self.deadline, self.buffer = name, deadline, b""
        try:
            self.process = subprocess.Popen([os.path.abspath(binary)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, cwd=root, bufsize=0)
        except OSError as e:
            raise Broken("the %s driver could not be started: %s" % (name, e))

    def send(self, data):
        try:
            self.process.stdin.write(data)
        except OSError:
            pass  # the driver has gone: the next read reports it

    def _more(self, what):
        """Reads whatever arrives next. False at end of output."""
        ready, _, _ = select.select([self.process.stdout], [], [], max(0, self.until - time.monotonic()))
        if not ready:
            raise Broken("the %s driver gave no %s within %s seconds" % (self.name, what, self.deadline))
        data = os.read(self.process.stdout.fileno(), 65536)
        self.buffer += data
        return bool(data)

    def line(self, what):
        """The next reply line, or None when the driver has closed its output."""
        self.until = time.monotonic() + self.deadline
        while b"\n" not in self.buffer:
            if not self._more(what):
                return None
        line, _, self.buffer = self.buffer.partition(b"\n")
        return line + b"\n"

    def finish(self):
        """Closes the driver's input. Returns what it still wrote and its exit status."""
        self.until = time.monotonic() + self.deadline
        try:
            self.process.stdin.close()
        except OSError:
            pass
        while self._more("end of output"):
            pass
        try:
            status = self.process.wait(max(0.1, self.until - time.monotonic()))
        except subprocess.TimeoutExpired:
            raise Broken("the %s driver closed its output but did not exit within %s seconds" % (self.name, self.deadline))
        rest, self.buffer = self.buffer, b""
        return rest, status

    def stop(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait()
        for pipe in (self.process.stdin, self.process.stdout):
            try:
                pipe.close()
            except OSError:
                pass


def run(name, binary, requests, root, inputs, loose, deadline):
    """Sends the requests one at a time. Returns the reply lines as bytes."""
    offset = 0 if inputs is None else 1
    driver = Driver(name, binary, root, deadline)
    replies = []
    try:
        if inputs is not None:
            requests = [dict({"op": "fix_inputs"}, **inputs)] + requests
        for number, request in enumerate(requests):
            before = digest(tree(root)) if number - offset in loose else None
            driver.send(json.dumps(request).encode() + b"\n")
            line = driver.line("reply to request %d (%s)" % (number, request["op"]))
            if line is None:
                _, status = driver.finish()
                raise Broken("the %s driver stopped answering at request %d (%s), status %s" % (name, number, request["op"], status))
            if inputs is not None and number == 0:
                if not accepted(line):
                    raise Broken("the %s driver refused fix_inputs: %s" % (name, line.decode(errors="replace").strip()))
                continue
            if before is not None:
                if accepted(line):
                    raise Broken("the %s driver accepted request %d (%s), which every engine must refuse" % (name, number - offset, request["op"]))
                if digest(tree(root)) != before:
                    raise Broken("the %s driver refused request %d (%s) but changed its store" % (name, number - offset, request["op"]))
            replies.append(line)
        rest, status = driver.finish()
        if rest:
            raise Broken("the %s driver wrote %d bytes that answer no request" % (name, len(rest)))
        if status != 0:
            raise Broken("the %s driver exited with status %d" % (name, status))
    finally:
        driver.stop()
    return replies


def run_stream(name, binary, stream, root, deadline):
    """Gives one new driver the stream's text exactly. Returns everything it wrote and its exit status."""
    driver = Driver(name, binary, root, deadline)
    try:
        driver.send(stream["input"].encode("utf-8", "surrogatepass"))
        written = b""
        for number in range(stream.get("replies_before_end", 0)):
            # The input is still open here: a driver that waits for more before it answers runs into the deadline.
            line = driver.line("reply %d while its input was open" % number)
            if line is None:
                break
            written += line
        rest, status = driver.finish()
        return written + rest, status
    finally:
        driver.stop()


def accepted(line):
    try:
        return json.loads(line).get("ok") is True
    except (ValueError, AttributeError):
        raise Broken("a reply is not a JSON object: %s" % line.decode(errors="replace").strip()[:160])


def string_end(line, at):
    """The index after the JSON string that starts at `at`, or -1."""
    if line[at:at + 1] != b'"':
        return -1
    at += 1
    while at < len(line):
        if line[at] == 0x5C:
            at += 2
        elif line[at] == 0x22:
            return at + 1
        else:
            at += 1
    return -1


def value_end(line, at):
    """The index after the JSON value that starts at `at`, or -1. Enough of a scanner to step over a value."""
    if line[at:at + 1] == b'"':
        return string_end(line, at)
    depth = 0
    while at < len(line):
        c = line[at]
        if c == 0x22:
            at = string_end(line, at)
            if at < 0:
                return -1
            if depth == 0:
                return at
            continue
        if c in b"{[":
            depth += 1
        elif c in b"}]":
            if depth == 0:
                return at
            depth -= 1
            if depth == 0:
                return at + 1
        elif c in b",\n" and depth == 0:
            return at
        at += 1
    return -1


def masked(line):
    """The reply line with the text of its top-level "message" string emptied, and every other byte as it was.

    A line that is not an object written the way the drivers write one (no spaces between tokens) is returned
    unchanged, and so is one whose message is not a string: then the bytes themselves are compared."""
    if line[:1] != b"{":
        return line
    at = 1
    while at < len(line) and line[at:at + 1] == b'"':
        key_end = string_end(line, at)
        if key_end < 0 or line[key_end:key_end + 1] != b":":
            return line
        end = value_end(line, key_end + 1)
        if end < 0:
            return line
        if line[at:key_end] == b'"message"' and line[key_end + 1:key_end + 2] == b'"':
            return line[:key_end + 1] + b'""' + line[end:]
        if line[end:end + 1] != b",":
            return line
        at = end + 1
    return line


def tree(root):
    """Every path under root: 'dir', ('link', target) or ('file', bytes)."""
    found = {}
    for directory, names, files in os.walk(root):
        for entry in names + files:
            path = os.path.join(directory, entry)
            relative = os.path.relpath(path, root)
            if os.path.islink(path):
                found[relative] = ("link", os.readlink(path))
            elif os.path.isdir(path):
                found[relative] = "dir"
            else:
                with open(path, "rb") as f:
                    found[relative] = ("file", f.read())
    return found


def first_difference(a, b):
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return i
    return min(len(a), len(b))


def compare(base, other, requests, loose):
    """The differences of `other` from `base`, as lines of text, and counts for the receipt."""
    (base_name, base_replies, base_tree), (name, replies, other_tree) = base, other
    differences = []
    if len(replies) != len(base_replies):
        differences.append("replies: %s gave %d, %s gave %d" % (base_name, len(base_replies), name, len(replies)))
    same_bytes = 0
    for number, (x, y) in enumerate(zip(base_replies, replies)):
        same_bytes += x == y
        if number in loose:
            continue  # every engine refused and none changed its store: checked in run
        if masked(x) != masked(y):
            differences.append("reply %d (%s): %s %s | %s %s" % (number, requests[number]["op"], base_name, x.decode(errors="replace").strip()[:160], name, y.decode(errors="replace").strip()[:160]))
    for path in sorted(set(base_tree) | set(other_tree)):
        x, y = base_tree.get(path), other_tree.get(path)
        if x is None or y is None:
            differences.append("tree: %s exists only for %s" % (path, name if x is None else base_name))
        elif x != y:
            if isinstance(x, tuple) and isinstance(y, tuple) and x[0] == y[0] == "file":
                differences.append("tree: %s differs at byte %d (%s has %d bytes, %s has %d)" % (path, first_difference(x[1], y[1]), base_name, len(x[1]), name, len(y[1])))
            else:
                differences.append("tree: %s is not the same kind of entry, or links elsewhere" % path)
    files = [v for v in base_tree.values() if isinstance(v, tuple) and v[0] == "file"]
    counts = {"requests": len(requests), "replies_identical_but_for_message_text": sum(masked(x) == masked(y) for x, y in zip(base_replies, replies)),
              "replies_identical_bytes": same_bytes, "refusals_with_any_code": len(loose), "paths": len(base_tree), "files": len(files), "file_bytes": sum(len(v[1]) for v in files),
              "tree_sha256": {n: digest(t) for n, t in ((base_name, base_tree), (name, other_tree))}, "differences": len(differences)}
    return differences, counts


def digest(found):
    h = hashlib.sha256()
    for path in sorted(found):
        entry = found[path]
        h.update(path.encode() + b"\0")
        h.update(entry.encode() if entry == "dir" else entry[0].encode() + b"\0" + (entry[1] if entry[0] == "file" else entry[1].encode()))
        h.update(b"\0")
    return h.hexdigest()


def usage(message):
    """Anything that is not a difference between engines ends with status 2."""
    print(message, file=sys.stderr)
    sys.exit(2)


def check(trace_name, trace):
    """Refuses a trace that is not well formed, or that would compare nothing for a format it names."""
    if set(trace) == {"about", "streams"}:
        streams = trace["streams"]
        if not isinstance(streams, list) or not streams:
            usage("trace %s: no streams" % trace_name)
        for stream in streams:
            if not isinstance(stream, dict) or not {"about", "input"} <= set(stream) or set(stream) - {"about", "input", "replies_before_end"} or not isinstance(stream["input"], str):
                usage("trace %s: a stream wants about, input and at most replies_before_end" % trace_name)
        return
    if set(trace) != {"about", "formats", "requests"} or not trace["formats"] or any(f not in FORMATS for f in trace["formats"]) or len(set(trace["formats"])) != len(trace["formats"]):
        usage("trace %s: want about, formats (%s, each once) and requests, or about and streams" % (trace_name, ", ".join(FORMATS)))
    if not isinstance(trace["requests"], list) or any(not isinstance(q, dict) or not isinstance(q.get("op"), str) for q in trace["requests"]):
        usage("trace %s: every request wants an op" % trace_name)
    for fmt in trace["formats"]:
        # "only" can leave a format the trace names with nothing to send, and two empty runs would compare as the same.
        if not any(q.get("only", fmt) == fmt for q in trace["requests"]):
            usage("trace %s: no request applies to the %s format it names" % (trace_name, fmt))
    for q in trace["requests"]:
        if "only" in q and q["only"] not in trace["formats"]:
            usage("trace %s: a request is only for %s, which the trace does not name" % (trace_name, q["only"]))


def compare_streams(base, other, stream):
    """The differences for one stream, as lines of text."""
    (base_name, base_out, base_status), (name, out, status) = base, other
    differences = []
    if base_status != status:
        differences.append("status: %s exited with %s, %s with %s" % (base_name, base_status, name, status))
    x, y = base_out.splitlines(keepends=True), out.splitlines(keepends=True)
    if len(x) != len(y):
        differences.append("replies: %s gave %d, %s gave %d" % (base_name, len(x), name, len(y)))
    for number, (p, q) in enumerate(zip(x, y)):
        if masked(p) != masked(q):
            differences.append("reply %d: %s %s | %s %s" % (number, base_name, p.decode(errors="replace").strip()[:160], name, q.decode(errors="replace").strip()[:160]))
    return differences, len(x), sum(p == q for p, q in zip(x, y))


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--engine", action="append", required=True, metavar="NAME=PATH", help="a driver; the first is the reference; name one twice to compare an engine with itself")
    parser.add_argument("--trace", action="append", help="run only this trace (the file name without .json)")
    parser.add_argument("--format", action="append", choices=FORMATS, help="run only this storage format")
    parser.add_argument("--without-seam", action="store_true", help="do not send fix_inputs: for a driver that has no test seam yet")
    parser.add_argument("--deadline", type=float, default=DEADLINE, help="seconds a driver has for each reply and for its exit (default %(default)s)")
    parser.add_argument("--traces", default=TRACES, help="the directory of traces (default rust/differential/traces)")
    parser.add_argument("--receipt", help="write the counts of every comparison to this file, as JSON")
    parser.add_argument("--keep", help="keep the engines' directories under this directory")
    args = parser.parse_args()
    engines = [tuple(e.split("=", 1)) for e in args.engine]
    if len(engines) < 2 or any(len(e) != 2 for e in engines):
        usage("give at least two --engine NAME=PATH")
    if args.deadline <= 0:
        usage("--deadline wants a positive number of seconds")
    names = sorted(os.path.basename(p)[:-5] for p in glob.glob(os.path.join(args.traces, "*.json")))
    for wanted in args.trace or []:
        if wanted not in names:
            usage("no trace named %s (have %s)" % (wanted, ", ".join(names)))
    traces = []
    for trace_name in args.trace or names:
        try:
            with open(os.path.join(args.traces, trace_name + ".json"), encoding="utf-8") as f:
                trace = json.load(f)
        except (OSError, ValueError) as e:
            usage("trace %s: %s" % (trace_name, e))
        if not isinstance(trace, dict):
            usage("trace %s: want an object" % trace_name)
        check(trace_name, trace)
        traces.append((trace_name, trace))
    receipt, different, broken, ran = [], 0, 0, 0

    def report(label, other, base, differences, summary):
        nonlocal different
        if differences:
            different += 1
            print("DIFFER  %s: %s against %s, %d differences" % (label, other, base, len(differences)))
            for line in differences[:12]:
                print("        " + line)
            if len(differences) > 12:
                print("        ... and %d more" % (len(differences) - 12))
        else:
            print("same    %s: %s against %s: %s" % (label, other, base, summary))

    for trace_name, trace in traces:
        if "streams" in trace:
            for number, stream in enumerate(trace["streams"]):
                ran += 1
                label = "%s [stream %d: %s]" % (trace_name, number, stream["about"])
                holder = tempfile.mkdtemp(prefix="differential-", dir=args.keep) if args.keep else None
                with tempfile.TemporaryDirectory(prefix="differential-") as scratch:
                    top = holder or scratch
                    try:
                        results = []
                        for index, (name, binary) in enumerate(engines):
                            root = os.path.join(top, "%d-%s" % (index, name))
                            os.mkdir(root)
                            results.append((name,) + run_stream(name, binary, stream, root, args.deadline))
                    except Broken as e:
                        broken += 1
                        print("BROKEN  %s: %s" % (label, e))
                        receipt.append({"trace": trace_name, "stream": number, "about": stream["about"], "broken": str(e)})
                        continue
                for other in results[1:]:
                    differences, lines, identical = compare_streams(results[0], other, stream)
                    receipt.append({"trace": trace_name, "stream": number, "about": stream["about"], "reference": results[0][0], "engine": other[0],
                                    "status": {results[0][0]: results[0][2], other[0]: other[2]}, "replies": lines, "replies_identical_bytes": identical, "differences": len(differences)})
                    report(label, other[0], results[0][0], differences, "status %s, %d replies (%d identical as bytes)" % (results[0][2], lines, identical))
            continue
        for fmt in trace["formats"]:
            if args.format and fmt not in args.format:
                continue
            ran += 1
            label = "%s [%s]" % (trace_name, fmt)
            holder = tempfile.mkdtemp(prefix="differential-", dir=args.keep) if args.keep else None
            with tempfile.TemporaryDirectory(prefix="differential-") as scratch:
                top = holder or scratch
                try:
                    requests, loose = requests_of(trace, fmt)
                    results = []
                    for index, (name, binary) in enumerate(engines):
                        root = os.path.join(top, "%d-%s" % (index, name))
                        os.mkdir(root)
                        replies = run(name, binary, requests, root, None if args.without_seam else INPUTS, loose, args.deadline)
                        results.append((name, replies, tree(root)))
                except Broken as e:
                    broken += 1
                    print("BROKEN  %s: %s" % (label, e))
                    receipt.append({"trace": trace_name, "format": fmt, "broken": str(e)})
                    continue
            for other in results[1:]:
                differences, counts = compare(results[0], other, requests, loose)
                receipt.append(dict({"trace": trace_name, "format": fmt, "reference": results[0][0], "engine": other[0], "seam": not args.without_seam}, **counts))
                report(label, other[0], results[0][0], differences, "%d replies (%d identical as bytes), %d files, %d bytes" % (
                    counts["requests"], counts["replies_identical_bytes"], counts["files"], counts["file_bytes"]))
    if args.receipt:
        with open(args.receipt, "w") as f:
            json.dump(receipt, f, indent=1)
            f.write("\n")
    if not ran:
        usage("nothing ran")
    print("%d comparisons: %d different, %d broken" % (len(receipt), different, broken))
    sys.exit(2 if broken else 1 if different else 0)


if __name__ == "__main__":
    main()
