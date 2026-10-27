// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::HashSet;

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use rezzer_derez::app::{App, Key};
use rezzer_derez::grid::{Dir, Pos, Tile};
use rezzer_derez::plan::{Plan, Rounds, builtin};
use rezzer_derez::round::Status;
use rezzer_derez::ui::draw;

fn rounds(name: &str, label: &str) -> Rounds {
    Rounds {
        name: name.to_string(),
        plans: vec![Plan {
            label: label.to_string(),
            ..builtin()
        }],
        note: None,
    }
}

fn render(app: &App, cols: u16, rows: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, cols, rows));
    draw(app, buf.area, &mut buf);
    buf
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

fn text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The two buffer cells of grid cell `p`.
fn cell(app: &App, buf: &Buffer, p: Pos) -> (String, String) {
    let area = app.grid_area().unwrap();
    let (x, y) = (area.x + 2 * p.x, area.y + p.y);
    (
        buf[(x, y)].symbol().to_string(),
        buf[(x + 1, y)].symbol().to_string(),
    )
}

fn clear_round(app: &mut App) {
    app.on_key(Key::Steer(Dir::Right));
    for _ in 0..2 {
        app.on_tick_with(|r, i| r.programs()[i].dir.opposite());
    }
    assert_eq!(app.round().unwrap().status(), Status::Cleared);
}

fn derez_round(app: &mut App) {
    for d in [Dir::Up, Dir::Left, Dir::Down, Dir::Right] {
        app.on_key(Key::Steer(d));
        app.on_tick_with(|r, i| r.programs()[i].dir);
    }
    assert_eq!(app.round().unwrap().status(), Status::Derezzed);
}

/// A round on the Grid in which the user program and one other program have moved three cells.
fn moved_app() -> App {
    let mut app = App::new(rounds("", "the Grid"), 1, 80, 30);
    app.on_key(Key::Steer(Dir::Right));
    for _ in 0..3 {
        app.on_tick_with(|r, i| r.programs()[i].dir);
    }
    assert_eq!(app.round().unwrap().status(), Status::Running);
    app
}

/// A grid cell with `tile` that is not a head.
fn find(app: &App, tile: Tile) -> Pos {
    let round = app.round().unwrap();
    let g = round.grid();
    let heads: Vec<Pos> = round.programs().iter().map(|p| p.pos).collect();
    (0..g.height())
        .flat_map(|y| (0..g.width()).map(move |x| Pos { x, y }))
        .find(|p| g.get(*p) == tile && !heads.contains(p))
        .unwrap_or_else(|| panic!("no {tile:?}"))
}

#[test]
fn status_line_shows_name_label_and_tally() {
    let mut app = App::new(rounds("report.xlsx", "sheet Costs"), 1, 80, 30);
    clear_round(&mut app);
    app.on_key(Key::Restart);
    derez_round(&mut app);
    app.on_key(Key::Restart);
    let buf = render(&app, 80, 30);
    assert_eq!(
        row(&buf, 0).trim_end(),
        "DEREZ  report.xlsx  sheet Costs  round 3  cleared 1  derezzed 1"
    );
    assert_eq!(
        row(&buf, 29).trim_end(),
        "arrows/WASD steer  p pause  r restart  q quit"
    );
}

#[test]
fn status_line_without_a_file_shows_the_grid() {
    let app = App::new(rounds("", "the Grid"), 1, 80, 30);
    let buf = render(&app, 80, 30);
    assert_eq!(
        row(&buf, 0).trim_end(),
        "DEREZ  the Grid  round 1  cleared 0  derezzed 0"
    );
}

#[test]
fn status_line_cleans_and_shortens_the_name() {
    let app = App::new(rounds("a\x1b]52;c;x\x07b", "page 1"), 1, 80, 30);
    let buf = render(&app, 80, 30);
    assert!(!text(&buf).chars().any(|c| c.is_control() && c != '\n'));
    assert_eq!(
        row(&buf, 0).trim_end(),
        "DEREZ  a?]52;c;x?b  page 1  round 1  cleared 0  derezzed 0"
    );

    let long = "n".repeat(300);
    let app = App::new(rounds(&long, "page\x1b[2J 1"), 1, 80, 30);
    let buf = render(&app, 80, 30);
    let line = row(&buf, 0);
    assert!(!line.chars().any(char::is_control));
    assert!(
        line.trim_end()
            .ends_with("...  page?[2J 1  round 1  cleared 0  derezzed 0"),
        "{line}"
    );
    assert!(line.starts_with("DEREZ  nnn"), "{line}");

    // A note that does not fit is left out; one that fits follows the tally.
    let mut r = rounds("x.bin", "the Grid");
    r.note = Some("x.bin gives no grid: a\x07. Using the Grid.".to_string());
    let app = App::new(r.clone(), 1, 80, 30);
    let buf = render(&app, 80, 30);
    assert_eq!(
        row(&buf, 0).trim_end(),
        "DEREZ  x.bin  the Grid  round 1  cleared 0  derezzed 0"
    );
    let app = App::new(r, 1, 120, 30);
    let buf = render(&app, 120, 30);
    assert_eq!(
        row(&buf, 0).trim_end(),
        "DEREZ  x.bin  the Grid  round 1  cleared 0  derezzed 0  \
         x.bin gives no grid: a?. Using the Grid."
    );

    // A terminal narrower than the fixed part: no panic, cut at the edge.
    let app = App::new(rounds(&long, "the Grid"), 1, 60, 20);
    let buf = render(&app, 60, 20);
    assert!(row(&buf, 0).starts_with("DEREZ  "));
    let tiny = render(&app, 10, 3);
    assert_eq!(row(&tiny, 0), "DEREZ  ...");
}

#[test]
fn wide_characters_count_twice() {
    // 40 wide characters are 80 columns, so the name must be shortened at 80 columns.
    let wide = "\u{8868}".repeat(40);
    let app = App::new(rounds(&wide, "sheet 1"), 1, 80, 30);
    let buf = render(&app, 80, 30);
    let line: String = row(&buf, 0);
    assert!(
        line.trim_end()
            .ends_with("...  sheet 1  round 1  cleared 0  derezzed 0"),
        "{line}"
    );
}

#[test]
fn tiles_differ_without_colour() {
    let app = moved_app();
    let buf = render(&app, 80, 30);
    let kinds = [Tile::Trail(0), Tile::Trail(1), Tile::Wall, Tile::Tower];
    let symbols: Vec<(String, String)> = kinds
        .iter()
        .map(|&t| cell(&app, &buf, find(&app, t)))
        .collect();
    assert_eq!(
        symbols,
        [
            ("█".to_string(), "█".to_string()),
            ("▓".to_string(), "▓".to_string()),
            ("▒".to_string(), "▒".to_string()),
            ("▀".to_string(), "▀".to_string()),
        ]
    );
    let distinct: HashSet<_> = symbols.iter().collect();
    assert_eq!(distinct.len(), 4);

    let round = app.round().unwrap();
    for p in round.programs() {
        assert_eq!(cell(&app, &buf, p.pos), ("◆".to_string(), "◆".to_string()));
    }
    // Empty cells: a dot every 4th column and row, else blank.
    let g = round.grid();
    for y in 0..g.height() {
        for x in 0..g.width() {
            let p = Pos { x, y };
            if g.get(p) == Tile::Empty {
                let want = if x % 4 == 0 && y % 4 == 0 { "·" } else { " " };
                assert_eq!(
                    cell(&app, &buf, p),
                    (want.to_string(), " ".to_string()),
                    "{p:?}"
                );
            }
        }
    }
}

#[test]
fn user_program_is_cyan_and_programs_orange() {
    let app = moved_app();
    let buf = render(&app, 80, 30);
    let area = app.grid_area().unwrap();
    let at = |p: Pos| &buf[(area.x + 2 * p.x, area.y + p.y)];
    let round = app.round().unwrap();

    let user = at(find(&app, Tile::Trail(0)));
    assert_eq!(user.fg, Color::Indexed(51));
    let other = at(find(&app, Tile::Trail(1)));
    assert_eq!(other.fg, Color::Indexed(208));
    assert_eq!(at(find(&app, Tile::Wall)).fg, Color::Indexed(31));
    assert_eq!(at(find(&app, Tile::Tower)).fg, Color::Indexed(250));
    assert_eq!(at(Pos { x: 4, y: 4 }).fg, Color::Indexed(236));

    let head = at(round.programs()[0].pos);
    assert_eq!(head.fg, Color::Indexed(51));
    assert!(head.modifier.contains(Modifier::BOLD));
    let head = at(round.programs()[1].pos);
    assert_eq!(head.fg, Color::Indexed(208));
    assert!(head.modifier.contains(Modifier::BOLD));
    // Foreground only, on the default background.
    assert_eq!(user.bg, Color::Reset);
    assert_eq!(head.bg, Color::Reset);
}

#[test]
fn derezzed_heads_show_a_cross_for_one_tick() {
    let mut app = App::new(rounds("", "the Grid"), 1, 80, 30);
    derez_round(&mut app);
    let user = app.round().unwrap().programs()[0].pos;
    let buf = render(&app, 80, 30);
    assert_eq!(cell(&app, &buf, user), ("✕".to_string(), " ".to_string()));
    let area = app.grid_area().unwrap();
    assert_eq!(
        buf[(area.x + 2 * user.x, area.y + user.y)].fg,
        Color::Indexed(51)
    );

    let mut app = App::new(rounds("", "the Grid"), 1, 80, 30);
    clear_round(&mut app);
    let round = app.round().unwrap();
    let other = round.programs()[1].pos;
    let buf = render(&app, 80, 30);
    assert_eq!(cell(&app, &buf, other), ("✕".to_string(), " ".to_string()));
    assert_eq!(
        buf[(area.x + 2 * other.x, area.y + other.y)].fg,
        Color::Indexed(208)
    );

    // A restart clears it.
    app.on_key(Key::Restart);
    let buf = render(&app, 80, 30);
    assert!(!text(&buf).contains('✕'));
}

#[test]
fn greeting_and_end_messages() {
    let app = App::new(rounds("", "the Grid"), 1, 80, 30);
    assert!(text(&render(&app, 80, 30)).contains("Greetings, program. Steer to start."));

    let mut app = App::new(rounds("", "the Grid"), 1, 80, 30);
    app.on_key(Key::Steer(Dir::Right));
    let buf = text(&render(&app, 80, 30));
    assert!(!buf.contains("Greetings"));

    clear_round(&mut app);
    let buf = text(&render(&app, 80, 30));
    assert!(
        buf.contains("End of Line.  r: next round  q: quit"),
        "{buf}"
    );

    let mut app = App::new(rounds("", "the Grid"), 1, 80, 30);
    derez_round(&mut app);
    let buf = text(&render(&app, 80, 30));
    assert!(buf.contains("Derezzed.  r: next round  q: quit"), "{buf}");

    let mut app = App::new(rounds("", "the Grid"), 1, 80, 30);
    let mut tick = 0;
    for d in [Dir::Up, Dir::Left, Dir::Down, Dir::Right] {
        app.on_key(Key::Steer(d));
        tick += 1;
        let back = tick == 4;
        app.on_tick_with(move |r, i| {
            let dir = r.programs()[i].dir;
            if back { dir.opposite() } else { dir }
        });
    }
    let buf = text(&render(&app, 80, 30));
    assert!(
        buf.contains("All derezzed.  r: next round  q: quit"),
        "{buf}"
    );
}

#[test]
fn too_small_says_so() {
    let app = App::new(rounds("", "the Grid"), 1, 59, 20);
    let buf = text(&render(&app, 59, 20));
    assert!(buf.contains("Make the terminal at least 60x20."), "{buf}");
    assert!(buf.contains("It is 59x20."), "{buf}");
    // Very small: no panic.
    let app = App::new(rounds("", "the Grid"), 1, 5, 1);
    render(&app, 5, 1);
    let app = App::new(rounds("", "the Grid"), 1, 0, 0);
    render(&app, 0, 0);
}

#[test]
fn a_buffer_smaller_than_the_app_does_not_panic() {
    // A frame drawn before a resize event arrives.
    let app = App::new(rounds("", "the Grid"), 1, 400, 100);
    let buf = render(&app, 80, 30);
    assert!(row(&buf, 0).starts_with("DEREZ"));
}

#[test]
fn draws_through_a_terminal() {
    let app = App::new(rounds("notes.md", "chapter 1"), 1, 80, 30);
    let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
    terminal
        .draw(|f| draw(&app, f.area(), f.buffer_mut()))
        .unwrap();
    let buf = terminal.backend().buffer();
    assert!(row(buf, 0).starts_with("DEREZ  notes.md  chapter 1  round 1"));
}
