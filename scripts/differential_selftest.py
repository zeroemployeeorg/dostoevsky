#!/usr/bin/env python3
"""Checks that scripts/differential.py reports what it must: run it against stand-in drivers whose replies are known.

    python3 scripts/differential_selftest.py

Standard library only. Each case names the reply bytes two stand-in drivers give and the exit status the runner must
end with: 0 the same, 1 different, 2 anything else. A stand-in driver answers every request line with its next reply.
"""
import json
import os
import stat
import subprocess
import sys
import tempfile

RUNNER = os.path.join(os.path.dirname(os.path.abspath(__file__)), "differential.py")
DRIVER = """#!/usr/bin/env python3
import sys, time
replies = %r
for line in sys.stdin.buffer:
    reply = replies.pop(0) if replies else b'{"ok":true}\\n'
    if reply == b"HANG":
        time.sleep(600)
    if reply == b"EXIT":
        sys.exit(1)
    sys.stdout.buffer.write(reply)
    sys.stdout.buffer.flush()
"""
FIXED = b'{"ok":true}\n'  # the reply to fix_inputs, which the runner sends first
ONE = {"about": "one request", "formats": ["legacy"], "requests": [{"op": "close"}]}

# name, trace, first driver's replies, second driver's replies, the status the runner must end with
CASES = [
    ("identical replies", ONE, [b'{"ok":true,"result":{"a":1}}\n'], [b'{"ok":true,"result":{"a":1}}\n'], 0),
    ("only the message text differs", ONE, [b'{"ok":false,"error":"invalid","message":"one"}\n'], [b'{"ok":false,"error":"invalid","message":"two \\" }"}\n'], 0),
    ("the message text differs and a result follows it", ONE, [b'{"ok":false,"error":"x","message":"one","result":{"size":1}}\n'], [b'{"ok":false,"error":"x","message":"2","result":{"size":1}}\n'], 0),
    ("the error code differs", ONE, [b'{"ok":false,"error":"invalid","message":"m"}\n'], [b'{"ok":false,"error":"limit","message":"m"}\n'], 1),
    ("true against 1", ONE, [b'{"ok":true,"result":{"reached":true}}\n'], [b'{"ok":true,"result":{"reached":1}}\n'], 1),
    ("1 against 1.0", ONE, [b'{"ok":true,"result":{"n":1}}\n'], [b'{"ok":true,"result":{"n":1.0}}\n'], 1),
    ("the order of members", ONE, [b'{"ok":true,"result":{"a":1,"b":2}}\n'], [b'{"ok":true,"result":{"b":2,"a":1}}\n'], 1),
    ("the escaping of a string", ONE, [b'{"ok":true,"result":{"a":"<"}}\n'], [b'{"ok":true,"result":{"a":"\\u003c"}}\n'], 1),
    ("a member given twice", ONE, [b'{"ok":true,"result":{"a":1}}\n'], [b'{"ok":true,"result":{"a":2,"a":1}}\n'], 1),
    ("white space between tokens", ONE, [b'{"ok":true}\n'], [b'{"ok": true}\n'], 1),
    ("a message that is not a string", ONE, [b'{"ok":false,"error":"x","message":"m"}\n'], [b'{"ok":false,"error":"x","message":7}\n'], 1),
    ("a message inside the result is not masked", ONE, [b'{"ok":true,"result":{"message":"a"}}\n'], [b'{"ok":true,"result":{"message":"b"}}\n'], 1),
    ("the result after a masked message differs", ONE, [b'{"ok":false,"error":"x","message":"m","result":{"size":1}}\n'], [b'{"ok":false,"error":"x","message":"m","result":{"size":2}}\n'], 1),
    ("a driver that never answers", ONE, [b'{"ok":true}\n'], [b"HANG"], 2),
    ("a driver that stops", ONE, [b'{"ok":true}\n'], [b"EXIT"], 2),
    ("a format with no request", {"about": "x", "formats": ["legacy", "segmented"], "requests": [{"op": "close", "only": "legacy"}]}, [], [], 2),
    ("no requests", {"about": "x", "formats": ["legacy"], "requests": []}, [], [], 2),
    ("no formats", {"about": "x", "formats": [], "requests": [{"op": "close"}]}, [], [], 2),
    ("a request without an op", {"about": "x", "formats": ["legacy"], "requests": [{"dir": "d"}]}, [], [], 2),
    ("a trace that is not JSON", None, [], [], 2),
    ("streams: the same bytes and status", {"about": "x", "streams": [{"about": "s", "input": "{}\n"}]}, [b'{"ok":true}\n'], [b'{"ok":true}\n'], 0),
    ("streams: one driver exits", {"about": "x", "streams": [{"about": "s", "input": "{}\n"}]}, [b'{"ok":true}\n'], [b"EXIT"], 1),
    ("streams: a reply wanted while the input is open never comes", {"about": "x", "streams": [{"about": "s", "input": "{}\n", "replies_before_end": 1}]}, [b'{"ok":true}\n'], [b"HANG"], 2),
    ("streams: none", {"about": "x", "streams": []}, [], [], 2),
]


def main():
    failed = 0
    for name, trace, first, second, want in CASES:
        with tempfile.TemporaryDirectory(prefix="differential-selftest-") as top:
            traces = os.path.join(top, "traces")
            os.mkdir(traces)
            with open(os.path.join(traces, "case.json"), "w") as f:
                f.write("{" if trace is None else json.dumps(trace))
            engines = []
            for label, replies in (("first", first), ("second", second)):
                path = os.path.join(top, label)
                lead = [] if trace is None or "streams" in trace else [FIXED]
                with open(path, "w") as f:
                    f.write(DRIVER % (lead + replies))
                os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
                engines += ["--engine", "%s=%s" % (label, path)]
            done = subprocess.run([sys.executable, RUNNER, "--traces", traces, "--deadline", "2"] + engines, capture_output=True, timeout=120)
            verdict = "ok  " if done.returncode == want else "FAIL"
            failed += done.returncode != want
            print("%s  %s: status %d, want %d" % (verdict, name, done.returncode, want))
            if done.returncode != want:
                print(done.stdout.decode(errors="replace") + done.stderr.decode(errors="replace"))
    for arguments in (["--engine", "one=/bin/true"], ["--engine", "a=/bin/true", "--engine", "b=/bin/true", "--trace", "no-such-trace"],
                      ["--engine", "a=/nonexistent/driver", "--engine", "b=/nonexistent/driver", "--trace", "seam-requests"]):
        done = subprocess.run([sys.executable, RUNNER] + arguments, capture_output=True, timeout=120)
        failed += done.returncode != 2
        print("%s  arguments %s: status %d, want 2" % ("ok  " if done.returncode == 2 else "FAIL", " ".join(arguments), done.returncode))
    print("%d cases, %d failed" % (len(CASES) + 3, failed))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
