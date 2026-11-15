# Parses a nextest log into the final failures: (binary, test, status, location, message).
import re
STATUS = re.compile(r"^\s*(?:TRY \d+ )?(PASS|FAIL|TIMEOUT|TMT|SIGSEGV|SIGABRT|SIGKILL|ABORT|LEAK|FLAKY|SKIP)\b.*?\] \(\s*\d+/\s*\d+\) (\S+) (\S+)\s*$")
PANIC = re.compile(r"^\s*thread '([^']*)' (?:\(\d+\) )?panicked at ([^:]+):(\d+)(?::\d+)?:\s*$")

def parse(path):
    lines = open(path, errors="replace").read().splitlines()
    last = {}
    order = []
    cur = None
    blocks = {}
    summary = False
    for i, l in enumerate(lines):
        if l.strip().startswith("Summary ["):
            summary = True
        m = STATUS.match(l)
        if m:
            st, binary, name = m.groups()
            if binary == "rezzer-read-pdf":
                binary = "lib"
            else:
                binary = binary.split("::", 1)[-1]
            key = (binary, name)
            if key not in last:
                order.append(key)
            last[key] = st
            if summary:
                cur = None
                if st != "PASS":
                    last[key] = st
                continue
            if st == "PASS":
                cur = None
            else:
                cur = key
                blocks[key] = []
            continue
        if l.startswith("------------") or summary:
            cur = None
            continue
        if cur is not None:
            blocks[cur].append(l)
    out = []
    for key in order:
        st = last[key]
        if st in ("PASS", "FLAKY", "SKIP"):
            continue
        loc, msg = "", ""
        b = blocks.get(key, [])
        for j, l in enumerate(b):
            m = PANIC.match(l)
            if m:
                loc = m.group(2) + ":" + m.group(3)
                body = []
                for x in b[j + 1:]:
                    if "RUST_BACKTRACE" in x or PANIC.match(x):
                        break
                    body.append(x.strip())
                msg = "\n".join(body)
                break
        out.append((key[0], key[1], "TIMEOUT" if st in ("TIMEOUT", "TMT") else st, loc, msg))
    return out
