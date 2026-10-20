// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use super::Ctx;
use super::sheet::{Note, Thread};
use super::workbook::Person;
use crate::xml::Ev;
use crate::{Issue, IssueKind};

/// Reads a legacy comments part. When `skip_placeholders` is set, the notes Excel writes next to
/// a threaded comment (author `tc={id}`) are left out: the thread carries the text.
pub(super) fn read_notes(ctx: &mut Ctx<'_, '_>, part: &str, skip_placeholders: bool) -> Vec<Note> {
    let mut authors: Vec<String> = Vec::new();
    let mut notes: Vec<Note> = Vec::new();
    let mut stack: Vec<String> = Vec::new();
    let mut author_text = String::new();
    let mut current: Option<(String, Option<usize>, String)> = None;
    let mut bad_author = false;
    ctx.scan(part, true, &mut |ev, _| match ev {
        Ev::Start(e) => {
            match e.name.as_str() {
                "author" => author_text.clear(),
                "comment" => {
                    current = Some((
                        e.attr("ref").unwrap_or("").to_string(),
                        e.attr("authorId").and_then(|v| v.parse().ok()),
                        String::new(),
                    ));
                }
                _ => {}
            }
            stack.push(e.name.clone());
        }
        Ev::Empty(e) if e.name == "comment" => notes.push(Note {
            cell: e.attr("ref").unwrap_or("").to_string(),
            author: e
                .attr("authorId")
                .and_then(|v| v.parse::<usize>().ok())
                .and_then(|i| authors.get(i).cloned()),
            text: String::new(),
        }),
        Ev::Text(t) => {
            if stack.last().is_some_and(|n| n == "t")
                && !stack.iter().any(|n| n == "rPh")
                && stack.iter().any(|n| n == "text")
            {
                if let Some((_, _, text)) = current.as_mut() {
                    text.push_str(t);
                }
            } else if stack.last().is_some_and(|n| n == "author") {
                author_text.push_str(t);
            }
        }
        Ev::End(name) => {
            stack.pop();
            match name {
                "author" => authors.push(std::mem::take(&mut author_text)),
                "comment" => {
                    if let Some((cell, author_id, text)) = current.take() {
                        let author = match author_id {
                            Some(i) => {
                                let a = authors.get(i).cloned();
                                if a.is_none() {
                                    bad_author = true;
                                }
                                a
                            }
                            None => None,
                        };
                        let placeholder = author.as_deref().is_some_and(|a| a.starts_with("tc="));
                        if !(skip_placeholders && placeholder) {
                            notes.push(Note { cell, author, text });
                        }
                    }
                }
                _ => {}
            }
        }
        _ => {}
    });
    if bad_author {
        ctx.issues.push(Issue::new(
            part,
            IssueKind::Damaged,
            "a comment names an author that is not in the author list",
        ));
    }
    notes
}

/// Reads a threaded comments part. Names come from the workbook's person list.
pub(super) fn read_threads(ctx: &mut Ctx<'_, '_>, part: &str, persons: &[Person]) -> Vec<Thread> {
    let mut threads: Vec<Thread> = Vec::new();
    let mut current: Option<(Thread, Option<String>)> = None;
    let mut in_text = false;
    let mut unknown: Vec<String> = Vec::new();
    ctx.scan(part, true, &mut |ev, _| match ev {
        Ev::Start(e) | Ev::Empty(e) if e.name == "threadedComment" => {
            let person_id = e.attr("personId").map(str::to_string);
            let thread = Thread {
                cell: e.attr("ref").unwrap_or("").to_string(),
                id: e.attr("id").unwrap_or("").to_string(),
                parent: e.attr("parentId").map(str::to_string),
                person: person_id
                    .as_ref()
                    .and_then(|id| persons.iter().find(|p| &p.id == id).map(|p| p.name.clone())),
                date: e.attr("dT").map(str::to_string),
                text: String::new(),
                done: e.flag("done"),
            };
            if let Some(id) = &person_id
                && thread.person.is_none()
                && !unknown.contains(id)
            {
                unknown.push(id.clone());
            }
            if matches!(ev, Ev::Start(_)) {
                current = Some((thread, person_id));
            } else {
                threads.push(thread);
            }
        }
        Ev::Start(e) if e.name == "text" && current.is_some() => in_text = true,
        Ev::Text(t) if in_text => {
            if let Some((thread, _)) = current.as_mut() {
                thread.text.push_str(t);
            }
        }
        Ev::End("text") => in_text = false,
        Ev::End("threadedComment") => {
            if let Some((thread, _)) = current.take() {
                threads.push(thread);
            }
        }
        _ => {}
    });
    for id in unknown {
        ctx.issues.push(Issue::new(
            part,
            IssueKind::Damaged,
            format!("person {id} is not in the person list"),
        ));
    }
    threads
}
