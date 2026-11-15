# usage: python3 -I p6x_words.py <base dir> <new dir>
# Word check of the three dump sets (corpus, forms, generated): every word of the base text
# must stand in the new output as often as in the base. Not checked: the frontmatter, the
# "Not read" section (loss wording), "Found by:" notes and status marks (:status[..]{..}).
# Words are compared after markup is taken off (emphasis, code fences, table pipes, heading
# marks, escapes), so a word in a table cell and the same word in a spatial block are equal.
# Merged-cell marks (← 〃) are no words either.
# Each missing word is put in one class:
#   glued   - a new word holds it together with more text (e.g. "PV-Beitrag6")
#   split   - it stands in the new text with white space inside it (e.g. "Menge 3")
#   value   - missing, holds a digit
#   text    - missing, a word without digits
# Prints Markdown: per set the files and words with residue by class, up to 5 example files
# per class with their words.
import html, os, re, sys, collections

SETS = ["corpus", "forms", "generated"]
CLASSES = ["glued", "split", "value", "text", "split at a hyphen", "hyphen dropped", "accepted"]
# no residue: "split at a hyphen" (se-lective now se- lective, as laid out) and "hyphen dropped"
# (adip-iscing now adipiscing): every letter still stands in the output
COUNTED = CLASSES[:4]
# Words the ledger rules no P6-X (a defect of the base read, or a mark that is no word now),
# by set, file and word, each with its reason (Task 19a-5a). They are listed, not counted.
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
ACCEPTED = _accepted("words")
FRONT = re.compile(r"\A---\n.*?\n---\n", re.S)
STATUS = re.compile(r":status\[[^\]]*\]\{[a-z]*\}")
EMPH = re.compile(r"(?<!\\)(\*\*|__|\*|_|`)")
ESC = re.compile(r"\\(.)")
EDGE = ".,:;!?()[]{}\"'*†‡"

NOTE = re.compile(r"^(Found by:|Merged cells:|\d+ footnotes? at the bottom of page \d+ (is|are) read )")
LINK = re.compile(r"(?<!\\)\[([^\]]*)\]\(([^)]*)\)")
FNREF = re.compile(r"\[\^[^\]]+\]:?")

def losses_of(t):
    # the "What" cells of the Not read table
    out, on = set(), False
    for l in t.splitlines():
        if l.startswith("## "):
            on = l.strip() == "## Not read"
            continue
        if on and l.startswith("| ") and not l.startswith("| Where") and not l.startswith("| ---"):
            cells = [c.strip() for c in re.split(r"(?<!\\)\|", l)[1:-1]]
            if len(cells) >= 2:
                out.add(ESC.sub(r"\1", cells[1]))
    return out

def text_of(t, drop_notes=True):
    t = FRONT.sub("", t, count=1)
    loss = losses_of(t) if drop_notes else set()
    out = []
    skip = fence = False
    for l in t.splitlines():
        if l.startswith("```"):
            fence = not fence
            continue
        if fence:
            # spatial and code blocks: raw text, no markup
            out.append(l)
            continue
        if l.startswith("## "):
            skip = l.strip() == "## Not read"
            if skip:
                continue
        if skip:
            continue
        if drop_notes and (NOTE.match(l) or ESC.sub(r"\1", l).strip() in loss):
            continue
        if re.match(r"^\|( *:?-{3,}:? *\|)+\s*$", l):
            continue
        l = re.sub(r"^#{1,6} ", "", l)
        l = STATUS.sub(" ", l)
        l = LINK.sub(r"\1", l)
        l = FNREF.sub(" ", l)
        l = EMPH.sub("", l)
        l = re.sub(r"(?<!\\)\|", " ", l)
        l = ESC.sub(r"\1", l)
        l = l.rstrip("\\")
        l = html.unescape(l)
        out.append(l)
    return "\n".join(out)

def words(t):
    c = collections.Counter()
    for w in t.split():
        w = w.strip(EDGE)
        # merged-cell marks of tables (← and 〃) are marks, not words
        if w and w != "|" and w != "-" and not re.fullmatch(r"[←〃↑]+", w):
            c[w] += 1
    return c

def classify(w, extra, flat):
    if "-" in w.strip("-") and w.replace("-", "") in extra:
        return "hyphen dropped"
    if "-" in w.strip("-") and re.search(
            "(?:" + "|".join(re.escape(w[:i + 1]) + r"\s+" + re.escape(w[i + 1:])
                             for i in range(len(w) - 1) if w[i] == "-") + ")", flat):
        return "split at a hyphen"
    for e in extra:
        if len(e) > len(w) and w in e:
            return "glued"
    if len(w) > 1:
        rx = r"\s+".join(re.escape(ch) for ch in w)
        if re.search(rx.replace(r"\s+", r"\s*", 0), flat) and re.search(
            "(?:" + "|".join(re.escape(w[:i]) + r"\s+" + re.escape(w[i:]) for i in range(1, len(w))) + ")", flat):
            return "split"
    return "value" if re.search(r"\d", w) else "text"

def check_set(base, new, name):
    b, n = os.path.join(base, name), os.path.join(new, name)
    files = residue_files = 0
    tokens = collections.Counter()
    files_by = collections.Counter()
    ex = collections.defaultdict(list)
    missing_files = []
    for root, _, fs in os.walk(b):
        for f in sorted(fs):
            if not f.endswith(".md"):
                continue
            pb = os.path.join(root, f)
            rel = os.path.relpath(pb, b)
            pn = os.path.join(n, rel)
            files += 1
            if not os.path.exists(pn):
                missing_files.append(rel)
                continue
            tb, tn = open(pb, errors="replace").read(), open(pn, errors="replace").read()
            if tb == tn:
                continue
            ob, on = text_of(tb), text_of(tn)
            wb, wn = words(ob), words(on)
            lost = wb - wn
            if not lost:
                continue
            extra = list((wn - wb).keys())
            per = collections.defaultdict(list)
            for w, k in lost.items():
                c = "accepted" if (name, rel, w) in ACCEPTED else classify(w, extra, on)
                tokens[c] += k
                per[c].append(w)
            if any(c in COUNTED for c in per):
                residue_files += 1
            for c, ws in per.items():
                files_by[c] += 1
                if len(ex[c]) < 5:
                    ex[c].append((rel, ws[:6]))
    return files, residue_files, tokens, files_by, ex, missing_files

def main():
    base, new = sys.argv[1], sys.argv[2]
    print("# Word check against " + base + "\n")
    print("Every word of the base text must stand in the new output as often (frontmatter, "
          "the Not read section, Found by notes and status marks left out).\n")
    print("| set | md files | files with residue | residue words | glued | split | value | text | split at a hyphen (not residue) | hyphen dropped (not residue) | new file missing |")
    print("|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    res = {}
    for s in SETS:
        r = check_set(base, new, s)
        res[s] = r
        files, rf, tok, fb, ex, miss = r
        print(f"| {s} | {files} | {rf + len(miss)} | {sum(tok[c] for c in COUNTED)} | " +
              " | ".join(f"{tok[c]} ({fb[c]} files)" for c in CLASSES) +
              f" | {len(miss)} |")
    for s in SETS:
        files, rf, tok, fb, ex, miss = res[s]
        print(f"\n## {s}\n")
        if miss:
            print(f"- new file missing ({len(miss)}): " + ", ".join(f"`{m}`" for m in miss[:5]))
        for c in CLASSES:
            if not fb[c]:
                continue
            print(f"- {c}: {tok[c]} words in {fb[c]} files")
            for rel, ws in ex[c]:
                print(f"  - `{rel}`: " + ", ".join(f"`{w}`" for w in ws))
    # machine-readable line for the summary
    print("\n<!-- residue " + " ".join(
        f"{s}={res[s][1] + len(res[s][5])}/{sum(res[s][2][c] for c in COUNTED)}/" +
        ",".join(f"{c.replace(' ', '_')}:{res[s][3][c]}" for c in CLASSES) for s in SETS) + " -->")

if __name__ == "__main__":
    main()
