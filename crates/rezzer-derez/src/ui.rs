// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Drawing an [`App`] into a buffer: the status line, the grid at two columns per cell, the
//! key line and the message.
//!
//! Every tile has its own glyph as well as its own colour, so the grid still reads without
//! colour (`NO_COLOR`, a 16-colour console). Colours are 256-colour indexes, not RGB, so
//! terminals without true colour show them too. Text from the file (its name, labels, notes)
//! has every control character and bidi formatting character replaced by `?`, so no escape
//! sequence reaches the terminal and nothing reorders the line.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::app::App;
use crate::grid::{Pos, Tile};
use crate::round::Round;

/// The user program.
const CYAN: Color = Color::Indexed(51);
/// The other programs.
const ORANGE: Color = Color::Indexed(208);
const WALL: Color = Color::Indexed(31);
const TOWER: Color = Color::Indexed(250);
const DOT: Color = Color::Indexed(236);
/// A dot on every this many columns and rows.
const DOT_EVERY: u16 = 4;

const KEYS: &str = "arrows/WASD steer  p pause  r restart  q quit";
const ELLIPSIS: &str = "...";
/// The columns a long name keeps before the label is shortened too.
const NAME_MIN: usize = 12;

/// Draws `app` into `area` of `buf`. Everything is clipped to `area` and to the buffer, so a
/// frame drawn before a resize arrives cannot panic.
pub fn draw(app: &App, area: Rect, buf: &mut Buffer) {
    let area = area.intersection(buf.area);
    if area.is_empty() {
        return;
    }
    if let (Some(round), Some(grid_area)) = (app.round(), app.grid_area()) {
        draw_grid(app, round, grid_area, area, buf);
    }
    let status = Line::from(vec![
        Span::styled("DEREZ", Style::new().fg(CYAN).add_modifier(Modifier::BOLD)),
        Span::raw(status_rest(app, area.width)),
    ]);
    buf.set_line(area.x, area.y, &status, area.width);
    if area.height >= 2 {
        buf.set_stringn(
            area.x,
            area.bottom() - 1,
            KEYS,
            usize::from(area.width),
            Style::new(),
        );
    }
    if let Some(message) = app.message() {
        draw_message(app, message, app.ready_note(), area, buf);
    }
}

/// The status line after `DEREZ`: the name, the label and the tally, then the note when the
/// whole line has room for it. Name and label are cleaned; when the line is too wide, the name
/// is shortened first (down to [`NAME_MIN`] columns), then the label, both with `...`, so the
/// round and the tally stay in view. The note never takes room from them.
fn status_rest(app: &App, width: u16) -> String {
    let width = usize::from(width);
    let t = app.tally();
    let tally = format!(
        "  round {}  cleared {}  derezzed {}",
        t.round, t.cleared, t.derezzed
    );
    let name = clean(app.name());
    let label = clean(app.label());
    // Room for the name and the label, with the two spaces before each; an empty one takes
    // no room and no spaces.
    let room = width.saturating_sub(text_width("DEREZ") + text_width(&tally));
    let name_part = if name.is_empty() {
        0
    } else {
        2 + text_width(&name).min(NAME_MIN)
    };
    let label = shorten(&label, room.saturating_sub(name_part + 2));
    let mut rest = String::new();
    if !name.is_empty() {
        let label_part = if label.is_empty() {
            0
        } else {
            2 + text_width(&label)
        };
        let name_room = room.saturating_sub(label_part + 2);
        rest.push_str("  ");
        rest.push_str(&shorten(&name, name_room));
    }
    if !label.is_empty() {
        rest.push_str("  ");
        rest.push_str(&label);
    }
    rest.push_str(&tally);
    if let Some(note) = app.note() {
        let note = clean(note);
        if text_width("DEREZ") + text_width(&rest) + 2 + text_width(&note) <= width {
            rest.push_str("  ");
            rest.push_str(&note);
        }
    }
    rest
}

/// `text` with every control character, and every bidi mark, embedding, override and isolate,
/// replaced by `?`, so a file name can neither send escapes nor reorder the status line.
fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() || is_bidi(c) { '?' } else { c })
        .collect()
}

/// The bidi formatting characters: marks (the Arabic letter mark too), embeddings and
/// overrides, and isolates.
fn is_bidi(c: char) -> bool {
    matches!(
        c,
        '\u{061C}' | '\u{200E}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
    )
}

/// `text` when it fits `room` columns, else as much of it as fits with `...` at the end.
fn shorten(text: &str, room: usize) -> String {
    if text_width(text) <= room {
        return text.to_string();
    }
    let keep = room.saturating_sub(ELLIPSIS.len());
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let mut b = [0; 4];
        let w = text_width(c.encode_utf8(&mut b));
        if used + w > keep {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push_str(ELLIPSIS);
    out
}

/// The display width of `text` in columns.
fn text_width(text: &str) -> usize {
    Span::raw(text).width()
}

fn colour_of(id: u8) -> Color {
    if id == 0 { CYAN } else { ORANGE }
}

/// The two glyphs and the style of a tile at `p`.
fn tile(t: Tile, p: Pos) -> ([&'static str; 2], Style) {
    match t {
        Tile::Empty if p.x.is_multiple_of(DOT_EVERY) && p.y.is_multiple_of(DOT_EVERY) => {
            (["·", " "], Style::new().fg(DOT))
        }
        Tile::Empty => ([" ", " "], Style::new()),
        Tile::Wall => (["▒", "▒"], Style::new().fg(WALL)),
        Tile::Tower => (["▀", "▀"], Style::new().fg(TOWER)),
        Tile::Trail(0) => (["█", "█"], Style::new().fg(CYAN)),
        Tile::Trail(_) => (["▓", "▓"], Style::new().fg(ORANGE)),
    }
}

/// Sets the two buffer cells of grid cell `p`, where they lie inside `clip`.
fn put(buf: &mut Buffer, grid_area: Rect, clip: Rect, p: Pos, glyphs: [&str; 2], style: Style) {
    let y = grid_area.y + p.y;
    for (i, glyph) in glyphs.into_iter().enumerate() {
        let x = grid_area.x + 2 * p.x + i as u16;
        if clip.contains((x, y).into())
            && let Some(cell) = buf.cell_mut((x, y))
        {
            cell.set_symbol(glyph).set_style(style);
        }
    }
}

fn draw_grid(app: &App, round: &Round, grid_area: Rect, clip: Rect, buf: &mut Buffer) {
    let grid = round.grid();
    for y in 0..grid.height() {
        for x in 0..grid.width() {
            let p = Pos { x, y };
            let (glyphs, style) = tile(grid.get(p), p);
            put(buf, grid_area, clip, p, glyphs, style);
        }
    }
    for program in round.programs() {
        let colour = colour_of(program.id);
        if !program.derezzed {
            let style = Style::new().fg(colour).add_modifier(Modifier::BOLD);
            // A glyph of its own, so the user program stands out without colour too.
            let head = if program.id == 0 { "◆" } else { "◇" };
            put(buf, grid_area, clip, program.pos, [head, head], style);
        } else if app.just_derezzed().contains(&program.id) {
            put(
                buf,
                grid_area,
                clip,
                program.pos,
                ["✕", " "],
                Style::new().fg(colour),
            );
        }
    }
}

/// The message: on the top lane of the grid, so the starts stay in view, with `note` (cleaned
/// and shortened to the grid width) on the row below it; or in the middle of the terminal while
/// it is too small, one sentence per row when it does not fit on one.
fn draw_message(app: &App, message: &str, note: Option<&str>, area: Rect, buf: &mut Buffer) {
    let style = Style::new().fg(CYAN).add_modifier(Modifier::BOLD);
    let (region, lines, top) = match app.grid_area() {
        Some(g) => {
            let region = g.intersection(area);
            let mut lines = vec![message.to_string()];
            if let Some(note) = note {
                // Room for the space on each side of the line.
                let room = usize::from(region.width).saturating_sub(2);
                lines.push(shorten(&clean(note), room));
            }
            (region, lines, g.y + 1)
        }
        None => {
            let lines: Vec<String> = if text_width(message) <= usize::from(area.width) {
                vec![message.to_string()]
            } else {
                message
                    .split_inclusive(". ")
                    .map(|s| s.trim().to_string())
                    .collect()
            };
            let height = lines.len() as u16;
            let top = area.y + area.height.saturating_sub(height) / 2;
            (area, lines, top)
        }
    };
    for (i, line) in lines.iter().enumerate() {
        let y = top + i as u16;
        if y < region.y || y >= region.bottom() {
            continue;
        }
        let padded = format!(" {line} ");
        let w = text_width(&padded).min(usize::from(region.width)) as u16;
        let x = region.x + (region.width - w) / 2;
        buf.set_stringn(x, y, &padded, usize::from(w), style);
    }
}
