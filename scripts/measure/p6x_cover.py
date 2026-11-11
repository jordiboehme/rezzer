# usage: python3 -I p6x_cover.py <engine dir>
# Cover check (Task 19 fix 1a): over the engine reports of the three dump sets, every run of a
# page is in exactly one region. A run in no region is excused only on a page whose layout
# stopped (its stop is named by the page's limit loss); a run in two regions never is.
import json, os, sys
root = sys.argv[1]
per_set = {}
lines = []
for s in ["corpus", "forms", "generated"]:
    d = os.path.join(root, s)
    if not os.path.isdir(d):
        continue
    files = pages = 0
    for dp, _, fs in os.walk(d):
        for f in sorted(fs):
            if not f.endswith(".json"):
                continue
            rel = os.path.relpath(os.path.join(dp, f), d)
            bad = 0
            for page in json.load(open(os.path.join(dp, f))):
                none = [] if page["stopped"] else page["uncovered"]
                if none or page["twice"]:
                    bad += 1
                    if len(lines) < 400:
                        lines.append(f"| {s} | {rel} | p{page['page']} | {len(none)} {none[:8]} | {len(page['twice'])} {page['twice'][:8]} | {page['issues']} |")
            if bad:
                files += 1
                pages += bad
    per_set[s] = (files, pages)
print("# Cover check\n")
print("Every run of a page in exactly one region (a run in none only on a page that stopped).\n")
for s, (f, p) in per_set.items():
    print(f"- {s}: {f} files, {p} pages")
print("\n| set | file | page | in no region | in two | issues |\n|---|---|---|---|---|---|")
print("\n".join(lines))
print("\n<!-- cover " + " ".join(f"{s}={f}/{p}" for s, (f, p) in per_set.items()) + " -->")
