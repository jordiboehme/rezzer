# usage: python3 -I p6x_triage.py <nextest log> [source root]
# Sorts every failing test of a nextest log into one bucket by rules on the assertion helper
# (the function the panic stands in), the test's name and its panic message. Prints a TSV:
# bucket, rule, binary, test, status, location, helper, message head.
# The rules are tried in order; the first that matches decides. No test is sorted by hand.
import ast, collections, os, re, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from p6x_parse import parse

BUCKETS = [
    "wrong pair",
    "lost pair or named loss",
    "glued or mixed cell",
    "dropped text",
    "lost header row or table",
    "dates and facts",
    "A finder internals",
    "B predicted 18b change",
    "time-out",
    "accepted (not P6-X)",
    "other",
]
P6X = BUCKETS[:6]

# Tests the Task 19 report names as the 18b group (bucket B, "update with reason").
B_NAMES = {"bold_label_then_value_is_no_heading", "amount_line_is_no_heading",
           "labels_without_colons_still_pair"}

SPATIAL_LOSS = r"is kept as laid out, as text in its columns"

# Failing tests the ledger rules no P6-X, by name, each with the signature of its ruled failure
# (a regex on the message) and its reason (Task 19a-5a). A failure of the same test with
# another message is sorted by the rules as before. Empty at 19a-5a: every test the ledger
# ruled no P6-X was rewritten to the engine's reading with its reason in the test.
ACCEPTED_TESTS = {
}
DURATION = re.compile(r"^\s*\d+(\.\d+)?(ns|µs|ms|s)\s*$")

def rules():
    # (name, bucket, test(binary, test, helper, msg) -> bool)
    m = lambda rx, fl=0: (lambda b, t, h, s: re.search(rx, s, fl) is not None)
    n = lambda rx: (lambda b, t, h, s: re.search(rx, t) is not None)
    both = lambda f, g: (lambda b, t, h, s: f(b, t, h, s) and g(b, t, h, s))
    w = lambda f: (lambda b, t, h, s, src: f(b, t, h, s))
    return [(name, bucket, f if f.__code__.co_argcount == 5 else w(f)) for name, bucket, f in [
        ("accepted by name (ACCEPTED_TESTS)", "accepted (not P6-X)",
         lambda b, t, h, s: t.split("::")[-1] in ACCEPTED_TESTS
         and re.search(ACCEPTED_TESTS[t.split("::")[-1]][0], s) is not None),
        ("nextest time-out", "time-out", lambda b, t, h, s: h == "<timeout>"),
        ("time: message is a duration", "time-out",
         lambda b, t, h, s: bool(DURATION.match(s)) or re.match(r"^\s*[\d.]+s against [\d.]+s\s*$", s) is not None),
        ("B: test named in the 18b group", "B predicted 18b change",
         lambda b, t, h, s: t.split("::")[-1] in B_NAMES),
        # dates and facts: a test of the dates module (or a test named for a date or a title)
        # whose failure is more than spatial losses added: a document date or fact that is
        # lost, wrong or said otherwise
        ("dates: a dates test or a date/fact named test, not only spatial losses added", "dates and facts",
         lambda b, t, h, s: (t.startswith("dates::") or re.search(r"(^|_)(document_)?dates?_(is|are|marks?|label|stands?|of_the_document)|facts?(_|$)", t.split("::")[-1]) is not None)
         and not spatial_only(b, t, h, s)),
        # wrong pair: the RightPair invariant or a "wrong pair" message
        ("msg RightPair / wrong pair", "wrong pair", m(r"RightPair|wrong pair")),
        # variants binary: the broken invariants of each variant ("| {Apart, LossSaid} |"),
        # the most severe decides
        ("variants: broken invariants (inv.rs Inv)", "", lambda b, t, h, s, src: variant_invs(s)),
        # the assertion's source: a Markdown pair row the output must hold
        ("src: md.contains(\"| label | value |\")", "lost pair or named loss",
         lambda b, t, h, s, src: re.search(r'contains\(\s*"\| [^|"]+ \| [^|"]+ \|"', src) is not None),
        # actual and expected tables or pairs of an assert_eq, compared cell by cell
        ("assert_eq of tables or pairs, compared", "",
         lambda b, t, h, s, src: compare_reads(b, t, h, s)),
        ("count of tables, rows or pairs short", "",
         lambda b, t, h, s, src: count_short(b, t, h, s, src)),
        # glued or mixed cells
        ("msg glued", "glued or mixed cell", m(r"\bglued\b")),
        ("msg holds texts of the cells", "glued or mixed cell", m(r"holds texts of the cells")),
        ("msg two blocks in", "glued or mixed cell", m(r"two blocks in ")),
        ("msg joins a cell / mixed", "glued or mixed cell", m(r"joins? (a|another|the next) cell|mixed|joined")),
        # dropped text: the ValueOnce invariant, values gone, empty reads of the limit paths
        ("msg ValueOnce", "dropped text",
         m(r"ValueOnce")),
        ("index out of bounds len 0, name of a pair", "lost pair or named loss",
         both(n(r"pair|label|field"), m(r"index out of bounds: the len is 0"))),
        ("name of a pair + nothing read (: [] [Loss)", "lost pair or named loss",
         both(n(r"pair"), m(r": \[\] \[Loss|: \[\]\n\[Loss|failed: \[\] \["))),
        # lost header rows or tables
        ("index out of bounds len 0, name of a table, row or header", "lost header row or table",
         both(n(r"table|header|row|cell|run"), m(r"index out of bounds: the len is 0"))),
        ("msg tables > 0 / every table empty", "lost header row or table",
         m(r"tables > 0|^\[\(0, \[\]\)(, \(\d+, \[\]\))*\]", re.M)),
        ("msg variant without a table (<variant>: [] [Loss)", "lost header row or table",
         m(r": \[\] \[Loss|: \[\]\n\[Loss")),
        ("msg no table / header [] / head no table / no one table", "lost header row or table",
         m(r"no (totals |one |ruled )?table|header \[\]|head no table|no table of|is in no table|"
           r"titles \[[^\]]*\] head|no cell |no header|no rows|neither a table")),
        ("msg no [<titles>] in [", "lost header row or table", m(r'no \["[^\]]*\] in \[')),
        # lost pairs or named losses
        ("msg ClearPairs / UnclearSaid / does not pair / no loss names", "lost pair or named loss",
         m(r"ClearPairs|UnclearSaid|does not pair|no loss names|no pair|not paired")),
        ("msg nor said / no loss says", "lost pair or named loss",
         m(r"nor said|no loss says|no loss name|without a loss|losses \[\]")),
        ("msg no <label>: <value> in [ / neither pairs / (label, value) in [", "lost pair or named loss",
         m(r"\bno [^\"\[\]:]{1,40}: [^\[\]\"]{1,60} in \[|neither pairs|\(\"[^\"]+\", \"[^\"]+\"\) in \[|paired (for|with|to)|"
           r"^\d+: [^:\[]+: [^\[]+ in \[\]", re.M)),
        ("helper or name of a loss + left 0 right 1", "lost pair or named loss",
         lambda b, t, h, s: re.search(r"loss", h + " " + t) is not None and re.search(r"left: 0\s+right: [1-9]", s) is not None),
        ("pairs expected, none read (left [] right [[[)", "lost pair or named loss",
         m(r"left: \[\]\s+right: \[\[\[")),
        # rules on the test's name, after every rule on the message alone
        ("name never join / stay(s) apart / in its cell + quoted join", "glued or mixed cell",
         both(n(r"never_join|stays?_apart|stays?_in_(its|their)_cell|joins_no_cell"),
              m(r"wrong|\[\]|no cell|\""))),
        ("name loses no read / keeps all text / every line", "dropped text",
         n(r"loses_no_read|keeps?_all_(the_)?text|keeps?_every_line|never_stops_the_text|keeps?_its_text")),
        ("name table/header/titles/rows (not hostile, no loss list) + nothing read", "lost header row or table",
         lambda b, t, h, s: b != "hostile" and "Loss {" not in s and "RegionNote" not in s
         and re.search(r"table|header|titles?|_rows?(_|$)", t) is not None
         and re.search(r"(^|: |left: )\[\](\s|$)|left: 0\s+right: [1-9]", s, re.M) is not None),
        ("Broken(Title): the title is no heading", "other", m(r"Broken\(\s*Title")),
        # cells or pairs that differ from the expected ones: the message cannot tell lost, wrong
        # or glued apart
        ("cells or pairs differ (cannot decide)", "other",
         both(m(r"assertion `left == right` failed"), m(r"\[\s*\[\s*\[?\s*\"[^\"]*\",\s"))),
        # bucket A: finder internals (pdf.* ids, finder order, layout unit tests of finders,
        # old loss wording of refused regions)
        ("A msg finder ids", "A finder internals",
         m(r"\"pdf\.[a-z_]+\"|\bpdf\.(lines|furniture|grid|ruled|tagged|columns|strips|boxes|pairs)\b|footer_band\.|spatial\.[a-z_]+")),
        ("A msg old refusal wording vs spatial wording", "A finder internals",
         both(m(SPATIAL_LOSS), m(r"right: \"(?!the region)"))),
        ("A lib layout unit test", "A finder internals",
         lambda b, t, h, s: b == "lib" and t.startswith("layout::tests::")),
        ("A grids found / segmented / RegionNote", "A finder internals",
         m(r"RegionNote|is segmented|found once")),
        # bucket B: the predicted 18b changes: spatial regions and their losses where
        # paragraphs, headings or no loss were read before
        ("B: only spatial losses added to what was expected", "B predicted 18b change", spatial_only),
    ]]

def helper_of(loc, root):
    # the function the panic location stands in and the assertion's source text, both read
    # from the source at the commit
    if not loc:
        return "", ""
    f, line = loc.rsplit(":", 1)
    short = f.split("/")[-1]
    path = os.path.join(root, f) if root else ""
    if not path or not os.path.isfile(path):
        return short, ""
    lines = open(path, errors="replace").read().splitlines()
    i = int(line) - 1
    src = []
    for l in lines[i:i + 10]:
        src.append(l.strip())
        if l.rstrip().endswith(";"):
            break
    for l in reversed(lines[: i + 1]):
        mm = re.match(r"\s*(?:pub(?:\([a-z]+\))?\s+)?fn\s+(\w+)", l)
        if mm:
            return short + "::" + mm.group(1), " ".join(src)
    return short, " ".join(src)

def loss_whats(s):
    return re.findall(r'what: "((?:[^"\\]|\\.)*)"', s)

def strings(s):
    return re.findall(r'"((?:[^"\\]|\\.)*)"', s)

def left_right(s):
    mm = re.search(r"\n\s*left: (.*)\n\s*right: (.*)\Z", s, re.S)
    return (mm.group(1).strip(), mm.group(2).strip()) if mm else (None, None)

def spatial_only(b, t, h, s):
    """B by structure: the failure is only spatial losses added to what was expected."""
    whats = loss_whats(s)
    l, r = left_right(s)
    if l is not None and re.fullmatch(r"\d+", l) and re.fullmatch(r"\d+", r) and whats:
        extra = sum(1 for w in whats if re.search(SPATIAL_LOSS, w))
        return extra > 0 and int(l) - extra == int(r)
    if l is not None and l.startswith("[") and r.startswith("["):
        ls = [x for x in strings(l) if not re.search(SPATIAL_LOSS, x)]
        rs = strings(r)
        return len(ls) < len(strings(l)) and collections.Counter(ls) == collections.Counter(rs)
    if l is None and whats and s.lstrip().startswith(("[Loss", "[ Loss", "[\nLoss")):
        return all(re.search(SPATIAL_LOSS, w) for w in whats)
    return False

def tables_of(v):
    # a read's tables: [(n, rows)], [rows], rows or [[pairs]] as lists of rows of strings
    if isinstance(v, list) and all(isinstance(x, tuple) and len(x) == 2 for x in v):
        v = [x[1] for x in v]
    if isinstance(v, list) and v and all(isinstance(x, list) and x and all(isinstance(y, str) for y in x) for x in v):
        v = [v]
    if not isinstance(v, list) or not all(isinstance(t, list) for t in v):
        return None
    for t in v:
        for r in t:
            if not isinstance(r, list) or not all(isinstance(c, str) for c in r):
                return None
    return v

VARIANT_INV = [
    (("RightPair", "WrongValue"), "wrong pair"),
    (("Apart", "Cells"), "glued or mixed cell"),
    (("ValueOnce", "CodeSaid"), "dropped text"),
    (("Table",), "lost header row or table"),
    (("ClearPairs", "UnclearSaid", "LossSaid"), "lost pair or named loss"),
]

def variant_invs(s):
    """The broken invariants listed per variant (Inv of tests/variants/inv.rs)."""
    sets = re.findall(r"\| \{([A-Za-z0-9, ]+)\} \|", s)
    if not sets:
        return None
    got = {x.strip() for st in sets for x in st.split(",")}
    for names, bucket in VARIANT_INV:
        if got & set(names):
            return bucket
    return None

def compare_reads(b, t, h, s):
    """Compares the actual (left) and expected (right) tables or pairs of an assert_eq."""
    l, r = left_right(s)
    if l is None:
        return None
    try:
        act, exp = tables_of(ast.literal_eval(l)), tables_of(ast.literal_eval(r))
    except Exception:
        return None
    if act is None or exp is None:
        return None
    pairs = re.search(r"pair|label|field", t) is not None
    arows = [r for tb in act for r in tb]
    erows = [r for tb in exp for r in tb]
    if not erows:
        return None
    if not arows:
        return "lost pair or named loss" if pairs else "lost header row or table"
    if any(len(r) == 2 for r in erows):
        av = {r[0]: r[1] for r in arows if len(r) == 2}
        if any(len(r) == 2 and r[0] in av and av[r[0]] != r[1] and r[1] for r in erows):
            return "wrong pair"
    cells = {c for r in arows for c in r}
    for r in erows:
        for x, y in zip(r, r[1:]):
            if x and y and (x + " " + y in cells or x + y in cells):
                return "glued or mixed cell"
    aset = {tuple(r) for r in arows}
    missing = [r for r in erows if tuple(r) not in aset]
    if missing:
        heads = {tuple(tb[0]) for tb in exp if tb}
        if all(tuple(r) in heads for r in missing):
            return "lost header row or table"
        return "lost pair or named loss" if pairs else "lost header row or table"
    return None

def count_short(b, t, h, s, src):
    """A count assertion of tables, rows or pairs that found fewer than expected."""
    l, r = left_right(s)
    if l is None or not (re.fullmatch(r"\d+", l) and re.fullmatch(r"\d+", r)):
        return None
    if int(l) >= int(r) or ".len()" not in src:
        return None
    if re.search(r"pair", src):
        return "lost pair or named loss"
    if re.search(r"table|rows|cells", src):
        return "lost header row or table"
    return None

def main():
    log = sys.argv[1]
    root = sys.argv[2] if len(sys.argv) > 2 else ""
    rs = rules()
    for binary, test, st, loc, msg in parse(log):
        h, src = ("<timeout>", "") if st == "TIMEOUT" else helper_of(loc, root)
        bucket, rule = "other", "no rule"
        for name, b, f in rs:
            got = f(binary, test, h, msg, src)
            if got:
                bucket, rule = (got if isinstance(got, str) else b), name
                break
        head = re.sub(r"\s+", " ", msg)[:220]
        print("\t".join([bucket, rule, binary, test, st, loc.split("/")[-1], h, head]))

if __name__ == "__main__":
    main()
