# usage: python3 -I p6x_kind.py <engine dir>
# Kind check (Task 19 fix 1a): over the engine reports of the three dump sets (written by the
# dumps stage with REZZER_ENGINE_DIR, see rezzer_read_pdf::engine_report), every region's kind
# matches the kind of the block it points to (its host; the bucket "kind mismatch"), and of
# each block its runs with text were written in (the row "kind mismatch by runs"). Figure and
# code regions are checked by host (Task 19b: their media block has an anchor, p<N>:m<K>, and
# is their host); by runs they are exempt, as the text in them is written by the walk
# (hosts.rs). A region whose host is no block of the page (it stands at the page) and runs
# written in no block are not checked. Reports written before the host word was added count
# by runs only.
import collections, json, os, sys
ALLOWED = {
    "heading": {"heading"}, "title_block": {"heading"},
    "paragraph": {"paragraph"}, "list": {"list"},
    "table": {"table"}, "label_value": {"table"}, "totals": {"table"},
    "spatial": {"spatial"},
    "header_band": {"page-header"}, "footer_band": {"page-footer"},
    "footnote_legend": {"section", "footnote"},
    "figure": {"media"}, "code": {"media"},
}
# Kinds checked by host only (Task 19b).
HOST_ONLY = {"figure", "code"}
root = sys.argv[1]
per_set = {}
by_host = {}
unchecked = collections.Counter()
lines = []
pairs = collections.Counter()
for s in ["corpus", "forms", "generated"]:
    d = os.path.join(root, s)
    if not os.path.isdir(d):
        continue
    files = regions = 0
    hfiles = hregions = 0
    for dp, _, fs in os.walk(d):
        for f in sorted(fs):
            if not f.endswith(".json"):
                continue
            rel = os.path.relpath(os.path.join(dp, f), d)
            bad = hbad = 0
            for page in json.load(open(os.path.join(dp, f))):
                for r in page["regions"]:
                    ok = ALLOWED.get(r["kind"])
                    if ok is None:
                        continue
                    unchecked[(s, r["kind"])] += r["blocks"].get("none", 0)
                    hw = r.get("host_word")
                    if hw is not None and hw not in ok:
                        hbad += 1
                        pairs[(s, r["kind"], "host " + hw)] += 1
                        if len(lines) < 400:
                            lines.append(f"| {s} | {rel} | p{page['page']} | {r['kind']} | {r['host']} (host {hw}) | {r['blocks']} |")
                    wrong = {} if r["kind"] in HOST_ONLY else {w: c for w, c in r["blocks"].items() if w != "none" and w not in ok}
                    if wrong:
                        bad += 1
                        for w, c in wrong.items():
                            pairs[(s, r["kind"], w)] += 1
                        if len(lines) < 400:
                            lines.append(f"| {s} | {rel} | p{page['page']} | {r['kind']} | {r['host']} | {r['blocks']} |")
            if bad:
                files += 1
                regions += bad
            if hbad:
                hfiles += 1
                hregions += hbad
    per_set[s] = (files, regions)
    by_host[s] = (hfiles, hregions)
print("# Kind check\n")
print("Every region's kind against the block it points to, and against the blocks its runs with text were written in (figure and code by host only).\n")
for s, (f, r) in by_host.items():
    print(f"- {s}, by host: {f} files, {r} regions")
for s, (f, r) in per_set.items():
    print(f"- {s}, by runs: {f} files, {r} regions")
print("\nRuns with ink written in no block (not checked by runs), by region kind: " + ", ".join(f"{s} {k} {v}" for (s, k), v in sorted(unchecked.items()) if v))
print("\n| set | region kind | written as | regions |\n|---|---|---|---:|")
for (s, k, w), c in sorted(pairs.items()):
    print(f"| {s} | {k} | {w} | {c} |")
print("\n| set | file | page | kind | host | blocks of its runs |\n|---|---|---|---|---|---|")
print("\n".join(lines))
print("\n<!-- kind " + " ".join(f"{s}={f}/{r}" for s, (f, r) in by_host.items()) + " -->")
print("<!-- kindruns " + " ".join(f"{s}={f}/{r}" for s, (f, r) in per_set.items()) + " -->")
