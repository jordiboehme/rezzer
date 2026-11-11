# usage: python3 -I p6x_summary.py <commit> <out dir> [full]
# Prints the P6-X summary table from <out dir>/triage.tsv and <out dir>/words.md; with "full"
# also the whole triage (every failing test with its rule) and the word check.
import collections, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from p6x_triage import BUCKETS, P6X

commit, out = sys.argv[1], sys.argv[2]
full = len(sys.argv) > 3 and sys.argv[3] == "full"
rows = [l.rstrip("\n").split("\t") for l in open(os.path.join(out, "triage.tsv")) if l.strip()]
by = collections.Counter(r[0] for r in rows)
words = open(os.path.join(out, "words.md")).read()
m = re.search(r"<!-- residue (.*?) -->", words)
res = {}
for part in m.group(1).split():
    name, rest = part.split("=", 1)
    files, toks, classes = rest.split("/", 2)
    res[name] = (int(files), int(toks), dict(x.split(":") for x in classes.split(",")))
fpath = os.path.join(out, "facts.md")
facts_md = open(fpath).read() if os.path.exists(fpath) else ""
fm = re.search(r"<!-- facts (.*?) -->", facts_md)
facts = {k: int(v) for k, v in (x.split("=") for x in fm.group(1).split())} if fm else {}
lpath = os.path.join(out, "links.md")
links_md = open(lpath).read() if os.path.exists(lpath) else ""
lm = re.search(r"<!-- links (.*?) -->", links_md)
links = {k: int(v) for k, v in (x.split("=") for x in lm.group(1).split())} if lm else {}
opath = os.path.join(out, "order.md")
order_md = open(opath).read() if os.path.exists(opath) else ""
om = re.search(r"<!-- order (.*?) -->", order_md)
order = {k: tuple(int(x) for x in v.split("/")) for k, v in (x.split("=") for x in om.group(1).split())} if om else {}
def tagged(name, tag):
    path = os.path.join(out, name)
    md = open(path).read() if os.path.exists(path) else ""
    m = re.search(rf"<!-- {tag} (.*?) -->", md)
    return md, ({k: tuple(int(x) for x in v.split("/")) for k, v in (x.split("=") for x in m.group(1).split())} if m else {})
kind_md, kind = tagged("kind.md", "kind")
_, kindruns = tagged("kind.md", "kindruns")
cover_md, cover = tagged("cover.md", "cover")
log = open(os.path.join(out, "tests.log"), errors="replace").read()
sm = re.findall(r"Summary \[.*", log)
tests_line = sm[-1].strip() if sm else "(no nextest summary line)"

p6x_tests = sum(by[b] for b in P6X)
p6x_files = (sum(v[0] for v in res.values()) + sum(facts.values()) + sum(links.values())
             + sum(v[0] for v in order.values()))
p6x_words = sum(v[1] for v in res.values())
print(f"# P6-X measure of {commit}\n")
print(f"Reader tests (`cargo nextest run -p rezzer-read-pdf --no-fail-fast`): {tests_line}\n")
print("| bucket | count |")
print("|---|---:|")
for b in P6X:
    print(f"| {b} | {by[b]} |")
for s in ["corpus", "forms", "generated"]:
    f, t, c = res[s]
    print(f"| word check residue, {s} (files / words) | {f} / {t} |")
for s in ["corpus", "forms", "generated"]:
    print(f"| dates and facts check, {s} (files) | {facts.get(s, 'not run')} |")
for s in ["corpus", "forms", "generated"]:
    print(f"| links and references check, {s} (files) | {links.get(s, 'not run')} |")
for s in ["corpus", "forms", "generated"]:
    o = order.get(s)
    print(f"| reading order check, {s} (files / lines) | {f'{o[0]} / {o[1]}' if o else 'not run'} |")
for s in ["corpus", "forms", "generated"]:
    o = kind.get(s)
    print(f"| kind mismatch, {s} (files / regions) | {f'{o[0]} / {o[1]}' if o else 'not run'} |")
for s in ["corpus", "forms", "generated"]:
    o = kindruns.get(s)
    print(f"| kind mismatch by runs, {s} (files / regions) | {f'{o[0]} / {o[1]}' if o else 'not run'} |")
for s in ["corpus", "forms", "generated"]:
    o = cover.get(s)
    print(f"| cover, {s} (files / pages) | {f'{o[0]} / {o[1]}' if o else 'not run'} |")
print(f"| **P6-X count** (tests in the six buckets + files with word residue, a changed date or fact, a lost link or reference, or a line out of order) | **{p6x_tests + p6x_files}** "
      f"({p6x_tests} tests + {p6x_files} files; {p6x_words} words) |")
for b in BUCKETS[6:]:
    print(f"| {b} | {by[b]} |")
print(f"| failing tests in all | {len(rows)} |")
others = [r for r in rows if r[0] == "other"]
if others:
    print("\n\"other\" (no rule decides, or the rule says the message cannot tell): " +
          ", ".join(f"`{r[3]}`" for r in others))
if full:
    print("\n## Triage, every failing test\n")
    print("Sorted by `p6x_triage.py`: the first rule that matches the helper, the name or the panic message decides.\n")
    for b in BUCKETS:
        rs = [r for r in rows if r[0] == b]
        if not rs:
            continue
        print(f"\n### {b} ({len(rs)})\n")
        print("| test | binary | rule | where | message head |")
        print("|---|---|---|---|---|")
        for r in rs:
            msg = r[7][:140].replace("|", "\\|")
            print(f"| `{r[3]}` | {r[2]} | {r[1]} | {r[6] or r[5]} | {msg} |")
    print("\n" + re.sub(r"^# ", "## ", words, count=1))
    if facts_md:
        print("\n" + re.sub(r"^# ", "## ", facts_md, count=1))
    if links_md:
        print("\n" + re.sub(r"^# ", "## ", links_md, count=1))
    if order_md:
        print("\n" + re.sub(r"^# ", "## ", order_md, count=1))
    for md in (kind_md, cover_md):
        if md:
            print("\n" + re.sub(r"^# ", "## ", md, count=1))
