# usage: python3 -I p6x_trace.py <nextest log with REZZER_TRACE_TODAY> <triage.tsv>
# For each failing test of the triage in a P6-X bucket: the classes of today's reading on its
# pages (tables by detector id, titles, refusals by family), from the TODAY trace lines of the
# test's output. Prints a TSV (bucket, test, binary, classes) and a summary per bucket.
import re, sys, collections
sys.path.insert(0, __import__("os").path.dirname(__import__("os").path.abspath(__file__)))
from p6x_parse import STATUS
log, tsv = sys.argv[1], sys.argv[2]
P6X = ["wrong pair", "lost pair or named loss", "glued or mixed cell", "dropped text",
       "lost header row or table", "dates and facts"]
want = {}
for l in open(tsv):
    r = l.rstrip("\n").split("\t")
    if r[0] in P6X:
        want[(r[2], r[3])] = r[0]
cls = collections.defaultdict(set)
cur = None
for l in open(log, errors="replace"):
    m = STATUS.match(l)
    if m:
        st, binary, name = m.groups()
        binary = "lib" if binary == "rezzer-read-pdf" else binary.split("::", 1)[-1]
        cur = (binary, name)
        continue
    if cur and "TODAY " in l:
        mm = re.search(r"TODAY p\d+ (table (\S+)|title (\w+)|refused (\w+)|unsure (\w+)|note (\w+))", l)
        if mm:
            if mm.group(2): cls[cur].add(mm.group(2).replace("pdf.", ""))
            elif mm.group(3): cls[cur].add("title")
            elif mm.group(4): cls[cur].add("refused." + mm.group(4))
            elif mm.group(5): cls[cur].add("unsure." + mm.group(5))
            elif mm.group(6): cls[cur].add("note." + mm.group(6))
by = collections.defaultdict(collections.Counter)
for key, b in sorted(want.items(), key=lambda x: (x[1], x[0])):
    c = ",".join(sorted(cls.get(key, {"(none)"})))
    print(f"{b}\t{key[1]}\t{key[0]}\t{c}")
    for x in cls.get(key, {"(none)"}):
        by[b][x] += 1
print()
for b in P6X:
    n = sum(1 for v in want.values() if v == b)
    print(f"# {b} ({n}): " + ", ".join(f"{k} {v}" for k, v in by[b].most_common()))
