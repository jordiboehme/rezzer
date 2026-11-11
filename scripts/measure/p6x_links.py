# usage: python3 -I p6x_links.py <base dir> <new dir>
# Links and references check of the three dump sets (Task 19a-5b): per Markdown file, the link
# targets (the "(target)" of a "[text](target)", images left out) and the footnote references
# ("[^id]" that is not a definition "[^id]:") of the base that the new output no longer has.
# Targets and references count as multisets. A block kept as laid out writes each of its
# targets once, with the lines it stands on, where the base wrote one link per line or style run
# of the same target: those cases are on the ACCEPTED list by file and target. A file that lost
# one or more counts once. Prints Markdown and a machine-readable line for the summary.
import collections, os, re, sys
SETS = ["corpus", "forms", "generated"]
LINK = re.compile(r"(?<!!)\[(?:[^\]\\]|\\.)*\]\(([^)\s]+)\)")
REF = re.compile(r"\[\^([^\]\s]+)\](?!:)")
# Lost targets or references the ledger rules no P6-X, by (set, file, kind, value), each with
# its reason (Task 19a-5b).
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
ACCEPTED = _accepted("links")
def items(t):
    c = collections.Counter()
    for m in LINK.finditer(t):
        c[("link", m.group(1))] += 1
    for m in REF.finditer(t):
        c[("ref", m.group(1))] += 1
    return c
def main():
    base, new = sys.argv[1], sys.argv[2]
    print("# Links and references check against " + base + "\n")
    print("| set | md files | files that lost a link or reference | lost targets | lost references | base targets / references |")
    print("|---|---:|---:|---:|---:|---|")
    res, ex = {}, {}
    for s in SETS:
        b, n = os.path.join(base, s), os.path.join(new, s)
        files = bad = 0
        lost = collections.Counter(); total = collections.Counter()
        ex[s] = []
        for root, _, fs in os.walk(b):
            for f in sorted(fs):
                if not f.endswith(".md"):
                    continue
                pb = os.path.join(root, f); rel = os.path.relpath(pb, b); pn = os.path.join(n, rel)
                files += 1
                cb = items(open(pb, errors="replace").read())
                for (k, _), v in cb.items():
                    total[k] += v
                if not cb or not os.path.exists(pn):
                    continue
                cn = items(open(pn, errors="replace").read())
                gone = [(k, v, cb[(k, v)] - cn[(k, v)]) for (k, v) in sorted(cb)
                        if cb[(k, v)] > cn[(k, v)] and (s, rel, k, v) not in ACCEPTED]
                if not gone:
                    continue
                bad += 1
                for k, _, d in gone:
                    lost[k] += d
                if len(ex[s]) < 10:
                    ex[s].append((rel, gone))
        res[s] = bad
        print(f"| {s} | {files} | {bad} | {lost['link']} | {lost['ref']} | {total['link']} / {total['ref']} |")
    for s in SETS:
        if ex[s]:
            print(f"\n## {s}\n")
            for rel, gone in ex[s]:
                print(f"- `{rel}`: " + "; ".join(f"{k} {v}" + (f" x{d}" if d > 1 else "") for k, v, d in gone[:8])
                      + (f"; and {len(gone) - 8} more" if len(gone) > 8 else ""))
    print("\n<!-- links " + " ".join(f"{s}={res[s]}" for s in SETS) + " -->")
if __name__ == "__main__":
    main()
