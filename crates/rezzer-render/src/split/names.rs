// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Slugs, unique names and titles for the units of a split.

use super::*;

/// A name from a title: lowercase ASCII letters and digits, any other run as one `-`, no
/// `-` at either end. Letters outside ASCII are left out (`Über` gives `ber`).
pub(crate) fn slug(text: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else if c.is_ascii() {
            gap = true;
        }
    }
    out
}

/// `slug` cut to at most `max` bytes, at its last `-` that keeps it within, else at `max`.
pub(super) fn cut_slug(slug: &str, max: usize) -> &str {
    if slug.len() <= max {
        return slug;
    }
    let head = &slug[..max];
    match head.rfind('-') {
        Some(at) if at > 0 => &head[..at],
        _ => head.trim_end_matches('-'),
    }
}

/// The part names given so far (decision 9): each unique, none reserved, in document order.
#[derive(Debug)]
pub(crate) struct Names {
    taken: HashSet<String>,
    /// The next number to try for a base and suffix, so many pieces of one base do not try
    /// every number again.
    next: HashMap<(String, String), usize>,
}

impl Names {
    pub(crate) fn new() -> Names {
        Names {
            taken: RESERVED.iter().map(|r| (*r).to_string()).collect(),
            next: HashMap::new(),
        }
    }

    /// The names a split gave its units, so a later name never takes one of them.
    pub(crate) fn taken<'n>(names: impl IntoIterator<Item = &'n str>) -> Names {
        let mut out = Names::new();
        out.taken.extend(names.into_iter().map(str::to_string));
        out
    }

    /// A free name from `base` (slugged) and `suffix` (already a slug part, such as
    /// `-rows-2-300`): `-2`, `-3` and so on when it is taken; `part-<order>` when both are
    /// empty. At most 60 bytes: the base is cut at a hyphen to make room, and a name still
    /// longer is cut at a hyphen and ends with `-<8 hex>` of the whole name, so it stays
    /// unique.
    pub(crate) fn take(&mut self, base: &str, suffix: &str, order: usize) -> String {
        let base = slug(base);
        let (base, suffix) = match (base.is_empty(), suffix.trim_start_matches('-')) {
            (true, "") => (format!("part-{order}"), String::new()),
            (true, rest) => (rest.to_string(), String::new()),
            (false, _) => (base, suffix.to_string()),
        };
        let key = (base.clone(), suffix.clone());
        let start = self.next.get(&key).copied().unwrap_or(1);
        for n in start.. {
            let tail = if n == 1 {
                suffix.clone()
            } else {
                format!("{suffix}-{n}")
            };
            let room = NAME_CAP.saturating_sub(tail.len());
            let name = format!("{}{tail}", cut_slug(&base, room));
            let mut name = name.trim_matches('-').to_string();
            if name.len() > NAME_CAP {
                name = hashed(&name);
            }
            if !name.is_empty() && self.taken.insert(name.clone()) {
                self.next.insert(key, n + 1);
                return name;
            }
        }
        unreachable!("some suffix is always free")
    }
}

/// `name` cut at a hyphen to make room for `-<8 hex>`, the FNV-1a hash of the whole name.
fn hashed(name: &str) -> String {
    let mut hash: u32 = 0x811c_9dc5;
    for b in name.bytes() {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    let head = cut_slug(name, NAME_CAP - 9).trim_end_matches('-');
    format!("{head}-{hash:08x}")
}

/// The names of a family's members: what the `n`-th of them (from 1) is titled.
pub(crate) fn family_title(title: &str, n: usize) -> String {
    if n <= 1 {
        title.to_string()
    } else {
        format!("{title}, part {n}")
    }
}

/// The title of Rezzer's own parts that list the other parts (ruling P5-AF), so a source
/// chapter named `Contents` keeps its name.
pub(crate) const LIST_TITLE: &str = "List of parts";

/// The whole overview when the main document cuts it.
pub(crate) const OVERVIEW_TITLE: &str = "Overview";

/// The whole `## Not read` list when the main document cuts it.
pub(crate) const NOT_READ_TITLE: &str = "Not read";

/// The blocks before the first border.
pub(crate) const START_TITLE: &str = "Start";

/// The footnote definitions no unit cites.
pub(crate) const FOOTNOTES_TITLE: &str = "Footnotes";

pub(crate) const PROPERTIES_TITLE: &str = "Properties";

pub(crate) const LOGIC_TITLE: &str = "Logic";

pub(crate) const ANNOTATIONS_TITLE: &str = "Annotations";

pub(crate) const CHANGES_TITLE: &str = "Changes";

pub(crate) const EMBEDDED_TITLE: &str = "Embedded";

pub(crate) const OUTLINE_TITLE: &str = "Outline";

/// Every title Rezzer gives its own parts, the constants the writers use.
#[cfg(test)]
pub(crate) const OWN_TITLES: [&str; 11] = [
    LIST_TITLE,
    OVERVIEW_TITLE,
    NOT_READ_TITLE,
    START_TITLE,
    FOOTNOTES_TITLE,
    PROPERTIES_TITLE,
    LOGIC_TITLE,
    ANNOTATIONS_TITLE,
    CHANGES_TITLE,
    EMBEDDED_TITLE,
    OUTLINE_TITLE,
];

/// How a unit's title is made, so the part writer can make it again once it knows which of
/// its own parts the document gets ([`finish_titles`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Naming {
    /// Rezzer made and titled it (`Start`, `Footnotes`, the trailing sections): its title
    /// never changes for a clash.
    pub(super) own: bool,
    /// The unit's title before its piece number, unique among the units of the source.
    pub(super) base: String,
    /// Where the unit (its first piece) lies.
    pub(super) location: String,
    /// Its piece number from 2, else 0 (a unit in one piece, or its first piece).
    pub(super) piece: usize,
}

/// A title as Crystalline compares it (L003): trimmed, in lower case.
pub(super) fn title_key(title: &str) -> String {
    title.trim().to_lowercase()
}

/// `title` with ` <suffix>` added, within [`TITLE_CAP`]: the title is cut to make room.
fn with_suffix(title: &str, suffix: &str) -> String {
    let room = TITLE_CAP.saturating_sub(suffix.chars().count() + 1);
    format!("{} {suffix}", capped(title, room))
}

/// The titles of the units of one document (title, location, own), made unique before their
/// pieces are numbered (rulings P5-AE and P5-AF), as Crystalline compares titles (L003).
/// Units of the source that share a title get their location in words,
/// `Abstract (pages 3 to 5)`; units that share title and location get ` (2)`, ` (3)` after
/// the first. Rezzer's own units keep theirs; a clash with them is [`finish_titles`]' job.
/// Each title stays within [`TITLE_CAP`], so the pieces' `, part <n>` fits the overhead.
pub(crate) fn unit_titles(units: &[(&str, &str, bool)]) -> Vec<String> {
    let mut count: HashMap<String, usize> = HashMap::new();
    for (title, _, own) in units {
        if !own {
            *count.entry(title_key(title)).or_insert(0) += 1;
        }
    }
    let mut taken: HashSet<String> = HashSet::new();
    units
        .iter()
        .map(|&(title, location, own)| {
            if own {
                return capped(title, TITLE_CAP);
            }
            let key = title_key(title);
            let location = location.trim();
            let located = if count.get(&key).is_some_and(|&n| n > 1)
                && !location.is_empty()
                && title_key(location) != key
            {
                with_suffix(title, &format!("({})", capped(location, 60)))
            } else {
                capped(title, TITLE_CAP)
            };
            let mut unique = located.clone();
            let mut n = 1;
            while !taken.insert(title_key(&unique)) {
                n += 1;
                unique = with_suffix(&located, &format!("({n})"));
            }
            unique
        })
        .collect()
}

/// Whether `key` is `own` or a piece of it (`logic, part 2`, `logic, sheet1`).
fn is_title_or_piece(key: &str, own: &str) -> bool {
    key == own
        || key
            .strip_prefix(own)
            .is_some_and(|rest| rest.starts_with(", "))
}

/// Whether `key` is `own` or one of its numbered pieces (`list of parts, part 2`), the
/// titles a part of `own` really gets.
fn is_title_or_numbered_piece(key: &str, own: &str) -> bool {
    key == own
        || key
            .strip_prefix(own)
            .and_then(|rest| rest.strip_prefix(", part "))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Writes the final title of every unit from its [`Naming`] (ruling P5-AF): a unit of the
/// source titled like a part Rezzer really writes for this document (one of its own units,
/// or one of `extra`, the parts the main document writer adds: `Overview`, `Not read`,
/// `List of parts`) gets its location in words, else the word `(section)`; then its piece
/// number. A title still used twice gets ` (2)`, ` (3)`, own units first; a unit of the
/// source also gets one when its title is one of `extra` or a numbered piece of it.
pub(crate) fn finish_titles(units: &mut [Unit], extra: &[&str]) {
    let present: Vec<String> = units
        .iter()
        .filter(|u| u.naming.own)
        .map(|u| title_key(&u.naming.base))
        .chain(extra.iter().map(|t| title_key(t)))
        .collect();
    for unit in units.iter_mut() {
        let n = &unit.naming;
        if n.base.is_empty() {
            continue;
        }
        let mut base = n.base.clone();
        let key = title_key(&base);
        if !n.own
            && present
                .iter()
                .any(|own| is_title_or_piece(&key, own) || is_title_or_piece(own, &key))
        {
            let location = n.location.trim();
            base = if !location.is_empty() && title_key(location) != key {
                with_suffix(&base, &format!("({})", capped(location, 60)))
            } else {
                with_suffix(&base, "(section)")
            };
        }
        unit.title = family_title(&base, n.piece);
    }
    let extra_keys: Vec<String> = extra.iter().map(|t| title_key(t)).collect();
    let mut taken: HashSet<String> = HashSet::new();
    for own in [true, false] {
        for unit in units.iter_mut().filter(|u| u.naming.own == own) {
            let base = unit.title.clone();
            let mut n = 1;
            loop {
                let key = title_key(&unit.title);
                let reserved = !own
                    && extra_keys
                        .iter()
                        .any(|e| is_title_or_numbered_piece(&key, e));
                if !reserved && taken.insert(key) {
                    break;
                }
                n += 1;
                unit.title = with_suffix(&capped(&base, TITLE_CAP), &format!("({n})"));
            }
        }
    }
}
