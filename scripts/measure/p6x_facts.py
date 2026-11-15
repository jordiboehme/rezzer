# usage: python3 -I p6x_facts.py <base dir> <new dir>
# Dates and facts check of the three dump sets: the frontmatter keys that carry the document's
# date and title facts (every key with "date", "temporal" or "title" in its name, but not the
# file's created and modified times) must read as in the base. A file whose such key is gone,
# new or changed counts once. Prints Markdown and a machine-readable line for the summary.
import os, re, sys, collections
SETS = ["corpus", "forms", "generated"]
FRONT = re.compile(r"\A---\n(.*?)\n---\n", re.S)
KEY = re.compile(r"date|temporal|title")
# Changed facts the ledger rules no P6-X, by set, file and key, each with its reason (Task
# 19a-5a). Left out of the comparison. Empty at 19a-5a: no fact differs from the base.
ACCEPTED_FACTS = {
}
def facts(t):
    m = FRONT.match(t)
    out = {}
    if not m:
        return out
    for l in m.group(1).splitlines():
        mm = re.match(r"^([a-z_]+):\s?(.*)$", l)
        if mm and KEY.search(mm.group(1)) and mm.group(1) not in ("created", "modified"):
            out[mm.group(1)] = mm.group(2)
    return out
def main():
    base, new = sys.argv[1], sys.argv[2]
    print("# Dates and facts check against " + base + "\n")
    print("| set | md files | files with a changed date or fact | lost | changed | new |")
    print("|---|---:|---:|---:|---:|---:|")
    res, ex = {}, {}
    for s in SETS:
        b, n = os.path.join(base, s), os.path.join(new, s)
        files = bad = 0
        kinds = collections.Counter()
        ex[s] = []
        for root, _, fs in os.walk(b):
            for f in sorted(fs):
                if not f.endswith(".md"):
                    continue
                pb = os.path.join(root, f); rel = os.path.relpath(pb, b); pn = os.path.join(n, rel)
                files += 1
                if not os.path.exists(pn):
                    continue
                fb, fn = facts(open(pb, errors="replace").read()), facts(open(pn, errors="replace").read())
                for k in [k for k in set(fb) | set(fn) if (s, rel, k) in ACCEPTED_FACTS]:
                    fb.pop(k, None); fn.pop(k, None)
                if fb == fn:
                    continue
                bad += 1
                d = []
                for k in sorted(set(fb) | set(fn)):
                    if k not in fn: kinds["lost"] += 1; d.append(f"{k} lost ({fb[k]})")
                    elif k not in fb: kinds["new"] += 1; d.append(f"{k} new ({fn[k]})")
                    elif fb[k] != fn[k]: kinds["changed"] += 1; d.append(f"{k}: {fb[k]} -> {fn[k]}")
                if len(ex[s]) < 8:
                    ex[s].append((rel, d))
        res[s] = bad
        print(f"| {s} | {files} | {bad} | {kinds['lost']} | {kinds['changed']} | {kinds['new']} |")
    for s in SETS:
        if ex[s]:
            print(f"\n## {s}\n")
            for rel, d in ex[s]:
                print(f"- `{rel}`: " + "; ".join(d))
    print("\n<!-- facts " + " ".join(f"{s}={res[s]}" for s in SETS) + " -->")
if __name__ == "__main__":
    main()
