# usage: python3 -I p6x_order.py <base dir> <new dir>
# Reading order check of the three dump sets (Task 19a-5d). Per page (the body sections of kind
# "page" of the JSON dump, matched by anchor), the text lines of the base (fix1) against the new
# output: both pages as word sequences, aligned by longest common subsequence (LCS). A base line
# is out of order when more than half of its words that still stand on the new page lie off the
# LCS (a word missing from the new page is the word check's business, not this one's).
# Lines: a paragraph or heading (split at its line breaks), a list item, a table cell, a line of
# a block kept as laid out (its pieces in column order). The page mark ("Page 1") is no line.
# A line out of order counts unless it is on the ACCEPTED list by set, file, page and line text, with
# the reason fix1 itself had it out of order. A file with a counted line counts once.
# Prints Markdown and a machine-readable line for the summary.
import collections, difflib, json, os, sys

SETS = ["corpus", "forms", "generated"]
EDGE = ".,:;!?()[]{}\"'*†‡"
# DP cells above this use difflib's matching blocks (not an exact LCS; the page is marked)
MAX_CELLS = 30_000_000
# Lines the ledger rules no P6-X: fix1 itself had them out of order. Key (set, file, page
# anchor, line text), so an entry cannot hide a later fault on another page of the file.
def _accepted(name):
    # The accepted list `name` of this check: `$P6X_ACCEPTED/<name>.json`, a list of
    # {"key": [...], "why": "..."}. The lists name files and quote text of the measured sets,
    # so they are kept beside the sets, not in the repo. No folder: nothing is accepted.
    import json as _json, os as _os
    folder = _os.environ.get("P6X_ACCEPTED")
    path = _os.path.join(folder, name + ".json") if folder else None
    if not path or not _os.path.exists(path):
        return {}
    return {tuple(e["key"]): e["why"] for e in _json.load(open(path, encoding="utf-8"))}
ACCEPTED = _accepted("order")

def inline_text(xs):
    # the text of a list of inlines, split at line breaks
    lines, cur = [], []
    for x in xs or []:
        t = x.get("type")
        if t == "line_break":
            lines.append("".join(cur)); cur = []
        elif t == "status":
            continue
        elif "inlines" in x:
            sub = inline_text(x["inlines"])
            if sub:
                cur.append(sub[0])
                for s in sub[1:]:
                    lines.append("".join(cur)); cur = [s]
        elif "text" in x:
            cur.append(x["text"])
    lines.append("".join(cur))
    return [l for l in lines if l.strip()]

def block_lines(b, out):
    t = b.get("type")
    if t in ("paragraph", "heading"):
        if b.get("role") == "page":
            return
        out.extend(inline_text(b.get("inlines")))
    elif t == "list":
        for item in b.get("items", []):
            for x in item:
                block_lines(x, out)
    elif t == "table":
        for row in b.get("rows", []):
            for cell in row:
                if "inlines" in cell:
                    out.extend(inline_text(cell["inlines"]))
                for x in cell.get("blocks", []):
                    block_lines(x, out)
    elif t == "spatial":
        for l in b.get("lines", []):
            ps = sorted(l.get("pieces", []), key=lambda p: p.get("col", 0))
            s = " ".join(p.get("text", "") for p in ps)
            if s.strip():
                out.append(s)
    elif "blocks" in b:
        for x in b["blocks"]:
            block_lines(x, out)
    elif "inlines" in b:
        out.extend(inline_text(b["inlines"]))

def pages(path):
    try:
        body = json.load(open(path, errors="replace")).get("body", [])
    except (ValueError, OSError):
        return {}
    out = {}
    for s in body:
        if s.get("type") == "section" and s.get("kind") == "page":
            ls = []
            for b in s.get("blocks", []):
                block_lines(b, ls)
            out[s.get("anchor")] = ls
    return out

def words(l):
    return [w for w in (x.strip(EDGE) for x in l.split()) if w]

def lcs_marks(a, b):
    # marks[i] True when a[i] is on an LCS of a and b; second value True when approximate
    n, m = len(a), len(b)
    marks = [False] * n
    p = 0
    while p < n and p < m and a[p] == b[p]:
        marks[p] = True; p += 1
    s = 0
    while s < n - p and s < m - p and a[n - 1 - s] == b[m - 1 - s]:
        marks[n - 1 - s] = True; s += 1
    A, B = a[p:n - s], b[p:m - s]
    if not A or not B:
        return marks, False
    if len(A) * len(B) > MAX_CELLS:
        sm = difflib.SequenceMatcher(None, A, B, autojunk=False)
        for blk in sm.get_matching_blocks():
            for k in range(blk.size):
                marks[p + blk.a + k] = True
        return marks, True
    ids = {}
    A = [ids.setdefault(w, len(ids)) for w in A]
    B = [ids.setdefault(w, len(ids)) for w in B]
    N, M = len(A), len(B)
    # direction table: 1 diagonal, 2 up, 0 left
    prev = [0] * (M + 1)
    dirs = []
    for i in range(N):
        cur = [0] * (M + 1)
        d = bytearray(M + 1)
        ai = A[i]
        for j in range(M):
            if ai == B[j]:
                cur[j + 1] = prev[j] + 1; d[j + 1] = 1
            elif prev[j + 1] >= cur[j]:
                cur[j + 1] = prev[j + 1]; d[j + 1] = 2
            else:
                cur[j + 1] = cur[j]
        dirs.append(d)
        prev = cur
    i, j = N, M
    while i > 0 and j > 0:
        d = dirs[i - 1][j]
        if d == 1:
            marks[p + i - 1] = True; i -= 1; j -= 1
        elif d == 2:
            i -= 1
        else:
            j -= 1
    return marks, False

def check_page(bl, nl):
    # base lines bl, new lines nl: the base lines out of order
    seq, owner = [], []
    for k, l in enumerate(bl):
        for w in words(l):
            seq.append(w); owner.append(k)
    new = [w for l in nl for w in words(l)]
    if seq == new:
        return [], False
    marks, approx = lcs_marks(seq, new)
    have = collections.Counter(new)
    present = collections.Counter(); off = collections.Counter()
    for w, k, on in zip(seq, owner, marks):
        if have[w] > 0:
            present[k] += 1
            if not on:
                off[k] += 1
    bad = [bl[k] for k in sorted(present) if off[k] * 2 > present[k]]
    return bad, approx

def check_set(base, new, name):
    b, n = os.path.join(base, name), os.path.join(new, name)
    res = dict(files=0, pages=0, lines=0, bad_files=0, counted=0, accepted=0, approx=0)
    ex = []
    acc = []
    for root, _, fs in os.walk(b):
        for f in sorted(fs):
            if not f.endswith(".json"):
                continue
            pb = os.path.join(root, f)
            rel = os.path.relpath(pb, b)
            pn = os.path.join(n, rel)
            md = rel[:-5] + ".md"
            res["files"] += 1
            if not os.path.exists(pn):
                continue
            pb_pages = pages(pb)
            pn_pages = pages(pn)
            counted_here = []
            for anchor, bl in pb_pages.items():
                res["pages"] += 1
                res["lines"] += len(bl)
                nl = pn_pages.get(anchor)
                if nl is None:
                    continue
                bad, approx = check_page(bl, nl)
                res["approx"] += approx
                for l in bad:
                    key = (name, md, anchor, l.strip())
                    if key in ACCEPTED:
                        res["accepted"] += 1
                        acc.append((md, anchor, l.strip(), ACCEPTED[key]))
                    else:
                        res["counted"] += 1
                        counted_here.append((anchor, l.strip()))
            if counted_here:
                res["bad_files"] += 1
                ex.append((md, counted_here))
    return res, ex, acc

def main():
    base, new = sys.argv[1], sys.argv[2]
    print("# Reading order check against " + base + "\n")
    print("Per page, the base lines against the new output by longest common subsequence of "
          "their words; a line is out of order when more than half of its words that still "
          "stand on the page lie off it.\n")
    print("| set | files | pages | base lines | files with lines out of order | lines out of order | accepted (fix1 out of order) | pages aligned approximately |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|")
    allr = {}
    for s in SETS:
        r, ex, acc = check_set(base, new, s)
        allr[s] = (r, ex, acc)
        print(f"| {s} | {r['files']} | {r['pages']} | {r['lines']} | {r['bad_files']} | "
              f"{r['counted']} | {r['accepted']} | {r['approx']} |")
    for s in SETS:
        r, ex, acc = allr[s]
        if not ex and not acc:
            continue
        print(f"\n## {s}\n")
        for md, ls in ex:
            print(f"- `{md}` ({len(ls)} lines)")
            for anchor, l in ls[:8]:
                print(f"  - {anchor}: `{l[:100]}`")
        for md, anchor, l, why in acc:
            print(f"- accepted: `{md}` {anchor} `{l[:80]}`: {why}")
    print("\n<!-- order " + " ".join(
        f"{s}={allr[s][0]['bad_files']}/{allr[s][0]['counted']}" for s in SETS) + " -->")

if __name__ == "__main__":
    main()
