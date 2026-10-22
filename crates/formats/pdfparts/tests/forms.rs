// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::path::PathBuf;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream, dictionary};
use pdfparts::script::{describe, field_refs};
use pdfparts::{Field, FieldKind, IssueKind, Limits, Pdf};

fn never() -> bool {
    false
}

fn open_bytes(bytes: &[u8]) -> Pdf {
    Pdf::open(bytes, None, &Limits::default(), &never).unwrap()
}

fn corpus(name: &str) -> Pdf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/corpus/pdf")
        .join(name);
    open_bytes(&std::fs::read(path).unwrap())
}

fn field<'a>(pdf: &'a Pdf, name: &str) -> &'a Field {
    let form = pdf.form().expect("a form");
    form.fields
        .iter()
        .find(|f| f.full_name == name)
        .unwrap_or_else(|| panic!("no field {name}"))
}

fn script<'a>(f: &'a Field, event: &str) -> &'a str {
    f.actions
        .iter()
        .find(|(e, _)| e == event)
        .map(|(_, s)| &**s)
        .unwrap_or_else(|| panic!("no {event} script on {}", f.full_name))
}

fn text(s: &str) -> Object {
    Object::string_literal(s)
}

/// A one-page document whose catalog gets `/AcroForm` from `build`. `build` may add objects
/// and gets the page id.
fn with_form(build: impl FnOnce(&mut Document, ObjectId) -> Dictionary) -> Pdf {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let acro = build(&mut doc, page_id);
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
        "AcroForm" => acro,
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    open_bytes(&out)
}

fn js(script: &str) -> Dictionary {
    dictionary! { "S" => "JavaScript", "JS" => text(script) }
}

#[test]
fn calculation_order_and_reads() {
    let pdf = corpus("pdfjs-text-field-calc.pdf");
    let form = pdf.form().unwrap();
    assert_eq!(form.calc_order, ["Mirror"]);
    assert!(form.need_appearances);
    assert!(!form.xfa);
    assert_eq!(form.fields.len(), 2);
    let source = field(&pdf, "Source");
    assert_eq!(source.kind, FieldKind::Text);
    assert_eq!(source.pages, [1]);
    assert!(source.flag_words.is_empty());
    let mirror = field(&pdf, "Mirror");
    assert_eq!(mirror.pages, [3]);
    assert_eq!(mirror.flags, 1);
    assert_eq!(mirror.flag_words, ["read-only"]);
    let calc = script(mirror, "calculate");
    assert_eq!(calc, r#"event.value = this.getField("Source").value;"#);
    assert_eq!(field_refs(calc).names, ["Source"]);

    // A field whose widgets are kids without a name is one terminal field.
    let pdf = corpus("pdfjs-acroform-calculation-order.pdf");
    let form = pdf.form().unwrap();
    assert_eq!(form.calc_order, ["group"]);
    assert_eq!(form.fields.len(), 1);
    assert_eq!(form.fields[0].kind, FieldKind::Text);
    assert_eq!(form.fields[0].pages, [1]);
}

#[test]
fn inherited_flags_and_type() {
    let pdf = with_form(|doc, page| {
        let parent = doc.new_object_id();
        let widget = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
            "P" => page,
        });
        let kid = doc.add_object(dictionary! {
            "T" => text("child"), "V" => text("value"), "Parent" => parent,
            "Kids" => vec![widget.into()], "TU" => text("Child name"),
        });
        let choice_kid = doc.add_object(dictionary! { "T" => text("pick"), "Parent" => parent });
        doc.objects.insert(
            parent,
            Object::Dictionary(dictionary! {
                "T" => text("parent"), "FT" => "Tx", "Ff" => 1,
                "Kids" => vec![kid.into(), choice_kid.into()],
            }),
        );
        let list = doc.add_object(dictionary! {
            "T" => text("list"), "FT" => "Ch", "Ff" => 2 + (1 << 21),
            "Opt" => vec![text("One"), Object::Array(vec![text("2"), text("Two")]), Object::Array(vec![text("x"), text("x")])],
            "V" => vec![text("One"), text("2")],
        });
        let list_kid = doc.add_object(dictionary! { "T" => text("sub"), "Parent" => list });
        if let Ok(Object::Dictionary(d)) = doc.get_object_mut(list) {
            d.set("Kids", vec![list_kid.into()]);
        }
        dictionary! { "Fields" => vec![parent.into(), list.into()] }
    });
    let form = pdf.form().unwrap();
    let names: Vec<&str> = form.fields.iter().map(|f| f.full_name.as_str()).collect();
    assert_eq!(names, ["parent.child", "parent.pick", "list.sub"]);
    let child = field(&pdf, "parent.child");
    assert_eq!(child.kind, FieldKind::Text);
    assert_eq!(child.flags, 1);
    assert_eq!(child.flag_words, ["read-only"]);
    assert_eq!(child.value.as_deref(), Some("value"));
    assert_eq!(child.alt_name.as_deref(), Some("Child name"));
    assert_eq!(child.pages, [1]);
    let pick = field(&pdf, "parent.pick");
    assert_eq!(pick.kind, FieldKind::Text);
    assert_eq!(pick.value, None);
    let sub = field(&pdf, "list.sub");
    assert_eq!(sub.kind, FieldKind::List);
    assert_eq!(sub.flag_words, ["required", "multi-select"]);
    let options: Vec<&str> = sub.options.iter().map(|o| &**o).collect();
    assert_eq!(options, ["One", "Two (2)", "x"]);
    assert_eq!(sub.value.as_deref(), Some("One, 2"));
}

#[test]
fn utf16_value_decoded() {
    let pdf = corpus("pypdf-libreoffice-form.pdf");
    let form = pdf.form().unwrap();
    assert_eq!(form.fields.len(), 8);
    assert!(form.need_appearances);
    let first = field(&pdf, "First Name");
    assert_eq!(first.value.as_deref(), Some("Alice"));
    assert_eq!(first.pages, [1]);
    assert_eq!(field(&pdf, "First Name_2").flag_words, ["multiline"]);
    let female = field(&pdf, "female");
    assert_eq!(female.kind, FieldKind::Radio);
    assert_eq!(female.value.as_deref(), Some("Off"));
    assert_eq!(female.flag_words, ["no-toggle-to-off"]);
    assert_eq!(field(&pdf, "gdpr").kind, FieldKind::Checkbox);
    let nat = field(&pdf, "Nationality");
    assert_eq!(nat.kind, FieldKind::Combo);
    assert_eq!(nat.options.len(), 7);
    assert_eq!(&*nat.options[1], "German");
}

#[test]
fn af_formats_in_words() {
    let pdf = corpus("pdfbox-sf1199a-direct-deposit.pdf");
    let amount = field(&pdf, "Allottment Amount");
    assert_eq!(amount.flags, 12582912);
    assert_eq!(amount.flag_words, ["do-not-spell-check", "do-not-scroll"]);
    let format = script(amount, "format");
    assert_eq!(format, r#"AFNumber_Format(2, 0, 0, 0, "$", true);"#);
    assert_eq!(
        describe(format).as_deref(),
        Some(r#"number, 2 decimals, "$" before the number"#)
    );
    assert_eq!(describe(script(amount, "keystroke")), None);
    let date = field(&pdf, "Bank Rep Date");
    assert_eq!(
        describe(script(date, "format")).as_deref(),
        Some("date mm/dd/yyyy")
    );
    assert_eq!(
        describe(script(date, "keystroke")).as_deref(),
        Some("date mm/dd/yyyy")
    );
    let phone = field(&pdf, "Phone");
    assert_eq!(
        describe(script(phone, "format")).as_deref(),
        Some("phone number")
    );
    assert!(amount.pages.len() >= 2, "{:?}", amount.pages);
}

#[test]
fn af_date_keystroke() {
    let pdf = corpus("pdfjs-dates-afdate.pdf");
    let d1 = field(&pdf, "Date1_af_date");
    assert_eq!(
        describe(script(d1, "keystroke")).as_deref(),
        Some("date dd-mmm-yy")
    );
    let d2 = field(&pdf, "Date2_af_date");
    assert_eq!(
        describe(script(d2, "format")).as_deref(),
        Some("time HH:MM:ss")
    );
    assert_eq!(describe(script(d2, "keystroke")), None);
    let d3 = field(&pdf, "Date3_af_date");
    assert_eq!(
        describe(script(d3, "format")).as_deref(),
        Some("date m/d/yyyy HH:MM")
    );
}

#[test]
fn xfa_is_reported() {
    let pdf = corpus("irs-f1040-2025.pdf");
    let form = pdf.form().unwrap();
    assert!(form.xfa);
    assert_eq!(form.fields.len(), 199);
    assert!(
        form.fields
            .iter()
            .all(|f| f.full_name.starts_with("topmostSubform[0]."))
    );
    assert!(form.fields.iter().all(|f| !f.pages.is_empty()));
    assert!(corpus("pypdf-outline.pdf").form().is_none());
    // An empty /Fields is still a form.
    assert!(
        corpus("pdfjs-annotation-highlight.pdf")
            .form()
            .unwrap()
            .fields
            .is_empty()
    );
}

#[test]
fn field_tree_cycle_is_damaged() {
    let pdf = with_form(|doc, _| {
        let a = doc.new_object_id();
        let b = doc
            .add_object(dictionary! { "T" => text("b"), "FT" => "Tx", "Kids" => vec![a.into()] });
        doc.objects.insert(
            a,
            Object::Dictionary(dictionary! { "T" => text("a"), "Kids" => vec![b.into()] }),
        );
        let c = doc.add_object(dictionary! { "T" => text("c"), "FT" => "Tx" });
        dictionary! { "Fields" => vec![a.into(), c.into(), c.into()] }
    });
    let form = pdf.form().unwrap();
    // b has a kid that is a field (a), so only c is terminal.
    let names: Vec<&str> = form.fields.iter().map(|f| f.full_name.as_str()).collect();
    assert_eq!(names, ["c"]);
    let damaged: Vec<_> = pdf
        .issues()
        .into_iter()
        .filter(|i| i.kind == IssueKind::Damaged && i.location == "form")
        .collect();
    assert_eq!(damaged.len(), 1, "{damaged:?}");
    assert!(
        damaged[0].what.starts_with("2 nodes are reached twice"),
        "{damaged:?}"
    );
}

#[test]
fn document_scripts_and_button_actions() {
    let pdf = corpus("pdfjs-js-buttons.pdf");
    let scripts = pdf.scripts();
    let doc: Vec<_> = scripts.iter().filter(|s| s.trigger == "document").collect();
    assert_eq!(doc.len(), 1);
    assert_eq!(doc[0].name.as_deref(), Some("callback"));
    assert_eq!(doc[0].kind, "javascript");
    let callback = doc[0].text.as_deref().unwrap();
    assert!(callback.starts_with("function callback(x)"));
    let refs = field_refs(callback);
    assert_eq!(refs.names, ["Field1"]);
    assert!(!refs.dynamic);
    let button = scripts
        .iter()
        .find(|s| s.trigger == "field Button3 activate")
        .unwrap();
    assert_eq!(
        field_refs(button.text.as_deref().unwrap()).names,
        ["Field1", "NoAct1", "Group2"]
    );
    // Radio widgets carry their own /A.
    let group: Vec<_> = scripts
        .iter()
        .filter(|s| s.trigger == "field Group1 activate")
        .map(|s| s.text.as_deref().unwrap())
        .collect();
    assert_eq!(group.len(), 4);
    assert_eq!(group[0], "callback(1);");
    assert_eq!(
        script(field(&pdf, "Button3"), "activate"),
        button.text.as_deref().unwrap()
    );
    // The outline's jump is not a script.
    assert!(scripts.iter().all(|s| s.kind != "goto"));

    let pdf = corpus("irs-f1040-2025.pdf");
    let doc: Vec<_> = pdf
        .scripts()
        .iter()
        .filter(|s| s.trigger == "document")
        .collect();
    assert_eq!(doc.len(), 3);
    assert_eq!(doc[0].name.as_deref(), Some("!ADBE::0100_VersChkStrings"));
    assert!(
        doc.iter()
            .all(|s| s.text.as_ref().is_some_and(|t| !t.is_empty()))
    );
    assert!(std::ptr::eq(pdf.scripts(), pdf.scripts()), "read once");
}

#[test]
fn open_page_and_document_actions_with_next_chains() {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let first = doc.new_object_id();
    let launch = doc.add_object(dictionary! {
        "S" => "Launch", "F" => text("calc.exe"), "Next" => first,
    });
    let uri = doc.add_object(dictionary! { "S" => "URI", "URI" => text("https://example.org/") });
    let mut open = js("app.alert(1);");
    open.set("Next", vec![launch.into(), uri.into()]);
    doc.objects.insert(first, Object::Dictionary(open));
    let stream = doc.add_object(Stream::new(dictionary! {}, b"this.print();".to_vec()));
    let submit = doc.add_object(dictionary! {
        "S" => "SubmitForm", "F" => dictionary! { "FS" => "URL", "F" => text("https://example.org/post") },
    });
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "AA" => dictionary! { "O" => js("pageOpen();"), "C" => submit },
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id, "OpenAction" => first,
        "AA" => dictionary! { "WP" => dictionary! { "S" => "JavaScript", "JS" => stream } },
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let pdf = open_bytes(&out);
    let got: Vec<(&str, &str, Option<&str>, Option<&str>)> = pdf
        .scripts()
        .iter()
        .map(|s| {
            (
                s.trigger.as_str(),
                s.kind.as_str(),
                s.target.as_deref().or(s.name.as_deref()),
                s.text.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("open", "javascript", None, Some("app.alert(1);")),
            ("open", "launch", Some("calc.exe"), None),
            ("open", "uri", Some("https://example.org/"), None),
            ("will-print", "javascript", None, Some("this.print();")),
            (
                "page-open",
                "javascript",
                Some("page 1"),
                Some("pageOpen();")
            ),
            (
                "page-close",
                "submit-form",
                Some("https://example.org/post"),
                None
            ),
        ]
    );
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.what.contains("a second time")),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn large_calculate_script_is_kept_as_text() {
    let big = format!("var x = 1;{}", " ".repeat(1 << 20));
    let pdf = with_form(|doc, page| {
        let f = doc.add_object(dictionary! {
            "T" => text("Total"), "FT" => "Tx", "Type" => "Annot", "Subtype" => "Widget", "P" => page,
            "AA" => dictionary! { "C" => js(&big) },
        });
        dictionary! { "Fields" => vec![f.into()], "CO" => vec![f.into()] }
    });
    let total = field(&pdf, "Total");
    assert_eq!(script(total, "calculate").len(), big.len());
    assert_eq!(pdf.form().unwrap().calc_order, ["Total"]);
    let s = &pdf.scripts()[0];
    assert_eq!(s.trigger, "field Total calculate");
    assert_eq!(s.text.as_ref().unwrap().len(), big.len());

    // Two fields share the script action: it is read once, and both fields share its text
    // within an object limit that holds one copy.
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages_id });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let action = doc.add_object(js(&big));
    let f = doc.add_object(dictionary! {
        "T" => text("Total"), "FT" => "Tx", "AA" => dictionary! { "C" => action },
    });
    let g = doc.add_object(dictionary! {
        "T" => text("Again"), "FT" => "Tx", "AA" => dictionary! { "C" => action },
    });
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id,
        "AcroForm" => dictionary! { "Fields" => vec![f.into(), g.into()] },
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let tight = Limits {
        max_object_bytes: (out.len() as u64) * 7 / 2,
        ..Limits::default()
    };
    let pdf = Pdf::open(&out, None, &tight, &never).unwrap();
    let fields = &pdf.form().unwrap().fields;
    assert_eq!(fields.len(), 2);
    assert!(std::sync::Arc::ptr_eq(
        &fields[0].actions[0].1,
        &fields[1].actions[0].1
    ));
    assert!(pdf.issues().is_empty(), "{:?}", pdf.issues());
}

#[test]
fn simple_calculate_and_range_check_in_a_form() {
    let pdf = with_form(|doc, page| {
        let a = doc.add_object(dictionary! { "T" => text("a"), "FT" => "Tx", "P" => page });
        let b = doc.add_object(dictionary! { "T" => text("b"), "FT" => "Tx", "P" => page });
        let total = doc.add_object(dictionary! {
            "T" => text("total"), "FT" => "Tx", "Ff" => 1, "P" => page,
            "AA" => dictionary! {
                "C" => js(r#"AFSimple_Calculate("SUM", new Array("a", "b"));"#),
                "V" => js("AFRange_Validate(true, 0, true, 100);"),
                "F" => js("AFNumber_Format(2);"),
            },
        });
        dictionary! { "Fields" => vec![a.into(), b.into(), total.into()], "CO" => vec![total.into()] }
    });
    let total = field(&pdf, "total");
    let events: Vec<&str> = total.actions.iter().map(|(e, _)| e.as_str()).collect();
    assert_eq!(events, ["calculate", "validate", "format"]);
    let calc = script(total, "calculate");
    assert_eq!(describe(calc).as_deref(), Some("sum of a, b"));
    assert_eq!(field_refs(calc).names, ["a", "b"]);
    assert_eq!(
        describe(script(total, "validate")).as_deref(),
        Some("between 0 and 100")
    );
    assert_eq!(describe(script(total, "format")), None);
    assert_eq!(pdf.form().unwrap().calc_order, ["total"]);
}

#[test]
fn actions_of_other_annotations_are_listed() {
    // A widget outside /Fields, a screen annotation and a link with /AA.
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let rect = || vec![0.into(), 0.into(), 10.into(), 10.into()];
    let widget = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Widget", "Rect" => rect(), "A" => js("app.alert(1)"),
    });
    let screen = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Screen", "Rect" => rect(), "A" => js("app.alert(2)"),
    });
    let link = doc.add_object(dictionary! {
        "Type" => "Annot", "Subtype" => "Link", "Rect" => rect(),
        "AA" => dictionary! { "E" => js("app.alert(3)") },
        "A" => js("app.alert(4)"),
    });
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Annots" => vec![widget.into(), screen.into(), link.into()],
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id,
        "AcroForm" => dictionary! { "Fields" => Vec::<Object>::new() },
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let pdf = open_bytes(&out);
    let got: Vec<(&str, &str)> = pdf
        .scripts()
        .iter()
        .map(|s| (s.trigger.as_str(), s.text.as_deref().unwrap_or("")))
        .collect();
    assert_eq!(
        got,
        [
            ("annotation page 1 activate", "app.alert(1)"),
            ("annotation page 1 activate", "app.alert(2)"),
            ("link page 1", "app.alert(4)"),
            ("annotation page 1 mouse-enter", "app.alert(3)"),
        ]
    );
}

#[test]
fn unreadable_and_shared_kids() {
    let mut doc = Document::with_version("1.7");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let child = doc.add_object(dictionary! {
        "T" => text("child"), "FT" => "Tx", "Subtype" => "Widget",
        "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
    });
    let kids = doc.add_object(vec![Object::Reference(child)]);
    let a = doc.add_object(dictionary! { "T" => text("a"), "Kids" => kids });
    let b = doc.add_object(dictionary! { "T" => text("b"), "Kids" => kids });
    // /Kids names an object that is not there.
    let c = doc.add_object(dictionary! {
        "T" => text("c"), "FT" => "Tx", "Subtype" => "Widget", "Kids" => (999, 0),
        "Rect" => vec![0.into(), 0.into(), 1.into(), 1.into()],
        "AA" => dictionary! { "K" => js("k") }, "A" => js("act"),
    });
    doc.objects.insert(
        page_id,
        Object::Dictionary(dictionary! {
            "Type" => "Page", "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Annots" => vec![child.into(), c.into()],
        }),
    );
    doc.objects.insert(
        pages_id,
        Object::Dictionary(
            dictionary! { "Type" => "Pages", "Kids" => vec![page_id.into()], "Count" => 1 },
        ),
    );
    let catalog = doc.add_object(dictionary! {
        "Type" => "Catalog", "Pages" => pages_id,
        "AcroForm" => dictionary! { "Fields" => vec![a.into(), b.into(), c.into()] },
    });
    doc.trailer.set("Root", catalog);
    let mut out = Vec::new();
    doc.save_to(&mut out).unwrap();
    let pdf = open_bytes(&out);
    let form = pdf.form().unwrap();
    let names: Vec<&str> = form.fields.iter().map(|f| f.full_name.as_str()).collect();
    assert_eq!(names, ["a.child", "c"]);
    let c = field(&pdf, "c");
    assert_eq!(c.pages, [1]);
    let actions: Vec<(&str, &str)> = c.actions.iter().map(|(e, s)| (e.as_str(), &**s)).collect();
    assert_eq!(actions, [("keystroke", "k"), ("activate", "act")]);
    let triggers: Vec<&str> = pdf.scripts().iter().map(|s| s.trigger.as_str()).collect();
    assert_eq!(triggers, ["field c keystroke", "field c activate"]);
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.what.starts_with("1 field names a /Kids array")),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn texts_many_fields_name_are_read_once() {
    let pdf = with_form(|doc, _| {
        let tu = doc.add_object(text("Shown name"));
        let v = doc.add_object(text("Value"));
        let js = doc.add_object(text("event.value = 1;"));
        let opt = doc.add_object(text("Choice"));
        let fields: Vec<Object> = (0..2)
            .map(|i| {
                doc.add_object(dictionary! {
                    "FT" => "Ch", "T" => text(&format!("f{i}")), "TU" => tu, "V" => v,
                    "Opt" => vec![Object::Reference(opt)],
                    "AA" => dictionary! { "C" => dictionary! { "S" => "JavaScript", "JS" => js } },
                })
                .into()
            })
            .collect();
        dictionary! { "Fields" => fields }
    });
    let form = pdf.form().unwrap();
    let [a, b] = [&form.fields[0], &form.fields[1]];
    let same = |x: &Option<std::sync::Arc<str>>, y: &Option<std::sync::Arc<str>>| {
        std::sync::Arc::ptr_eq(x.as_ref().unwrap(), y.as_ref().unwrap())
    };
    assert_eq!(a.alt_name.as_deref(), Some("Shown name"));
    assert!(same(&a.alt_name, &b.alt_name));
    assert!(same(&a.value, &b.value));
    assert!(std::sync::Arc::ptr_eq(&a.options[0], &b.options[0]));
    assert_eq!(&*a.actions[0].1, "event.value = 1;");
    assert!(std::sync::Arc::ptr_eq(&a.actions[0].1, &b.actions[0].1));
    // The scripts list shares the form's texts.
    let scripts = pdf.scripts();
    assert_eq!(scripts.len(), 2, "{scripts:?}");
    assert!(same(&scripts[0].text, &scripts[1].text));
}

#[test]
fn a_field_reached_through_hops_is_listed_once() {
    let pdf = with_form(|doc, _| {
        let f = doc.add_object(dictionary! { "T" => text("Once"), "FT" => "Tx" });
        let a = doc.add_object(Object::Reference(f));
        let b = doc.add_object(Object::Reference(a));
        dictionary! { "Fields" => vec![a.into(), b.into()], "CO" => vec![b.into()] }
    });
    let form = pdf.form().unwrap();
    assert_eq!(form.fields.len(), 1, "{:?}", form.fields);
    // The calculation order names the field through another chain.
    assert_eq!(form.calc_order, ["Once"]);
    assert!(
        pdf.issues()
            .iter()
            .any(|i| i.kind == IssueKind::Damaged && i.what.starts_with("1 node is reached twice")),
        "{:?}",
        pdf.issues()
    );
}

#[test]
fn an_unclosed_language_escape_in_a_field_name_keeps_its_text() {
    // UTF-16BE: ESC, then "Name" and no closing ESC.
    let mut raw = vec![0xFE, 0xFF, 0, 0x1B];
    for b in b"Name" {
        raw.extend([0, *b]);
    }
    let pdf = with_form(|doc, _| {
        let f = doc.add_object(dictionary! {
            "FT" => "Tx",
            "T" => Object::String(raw, lopdf::StringFormat::Hexadecimal),
        });
        dictionary! { "Fields" => vec![f.into()] }
    });
    assert_eq!(field(&pdf, "Name").kind, FieldKind::Text);
    assert!(
        pdf.issues().iter().any(|i| i.kind == IssueKind::Damaged
            && i.location == "form"
            && i.what.contains("language escape")),
        "{:?}",
        pdf.issues()
    );
}
