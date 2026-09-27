use ttfx::engine::terminal::{Terminal, TerminalConfig};
use ttfx::utils::geometry::Coord;
use ttfx::utils::graphics::{Color, ColorPair};

// Independent reference: sort and repaint the entire frame, with no caches.
fn reference(terminal: &Terminal) -> Vec<String> {
    let width = terminal.visible_right.max(0) as usize;
    let height = terminal.visible_top.max(0) as usize;
    let mut cells = vec![" "; width * height];
    let mut characters: Vec<_> = terminal.arena.iter().filter(|ch| ch.is_visible).collect();
    characters.sort_by_key(|ch| (ch.layer, ch.character_id));
    for ch in characters {
        let row = ch.motion.current_coord.row + terminal.canvas_row_offset;
        let column = ch.motion.current_coord.column + terminal.canvas_column_offset;
        if terminal.visible_bottom <= row
            && row <= terminal.visible_top
            && terminal.visible_left <= column
            && column <= terminal.visible_right
        {
            cells[(row - 1) as usize * width + (column - 1) as usize] = ch
                .animation
                .current_character_visual
                .formatted_symbol
                .as_str();
        }
    }
    (0..height)
        .map(|row| cells[row * width..(row + 1) * width].concat())
        .collect()
}

fn check(terminal: &mut Terminal) {
    let rows = reference(terminal);
    let expected = rows
        .iter()
        .rev()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(terminal.get_formatted_output_string(), expected);
    assert_eq!(
        terminal.get_formatted_output_string(),
        expected,
        "unchanged frame"
    );
    terminal.update_terminal_state();
    assert_eq!(terminal.terminal_state, rows);
}

#[test]
fn incremental_frames_match_full_repaint() {
    let mut terminal = Terminal::new(
        "abc\ndef",
        TerminalConfig {
            canvas_width: 13,
            canvas_height: 5,
            ignore_terminal_dimensions: true,
            ..TerminalConfig::default()
        },
    )
    .unwrap();
    let mut ids = terminal.input_characters.clone();
    for i in 0..30 {
        ids.push(terminal.add_character("█", Coord::new(i % 13 + 1, i % 5 + 1)));
    }
    for &id in &ids {
        terminal.set_character_visibility(id, true);
        // All characters start crowded into one cell.
        terminal.arena[id.0 as usize].motion.current_coord = Coord::new(2, 2);
    }
    check(&mut terminal);
    // Hide each successive winner, including hide/show before a render.
    for &id in ids.iter().rev() {
        terminal.set_character_visibility(id, false);
        check(&mut terminal);
        terminal.set_character_visibility(id, true);
        terminal.set_character_visibility(id, false);
    }
    let long_symbol = "long visual ".repeat(12);
    let mut random = 42u64;
    for tick in 0..2000 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let id = ids[(random >> 32) as usize % ids.len()];
        let ch = &mut terminal.arena[id.0 as usize];
        match tick % 7 {
            0 => {
                ch.motion.current_coord =
                    Coord::new((random % 19) as i64 - 3, ((random >> 8) % 9) as i64 - 2)
            }
            1 => ch.layer = (random % 9) as i64 - 4,
            2 => {
                let symbol = ["é", "█", " ", "λ", long_symbol.as_str()][random as usize % 5];
                ch.animation.set_appearance(
                    "x",
                    false,
                    Some(symbol),
                    Some(ColorPair::new(
                        Some(Color::from_hex("123456").unwrap()),
                        None,
                    )),
                );
            }
            3 => {
                let visible = !ch.is_visible;
                terminal.set_character_visibility(id, visible);
            }
            4 => ch.motion.current_coord = Coord::new(2, 2),
            5 => {
                terminal.canvas_column_offset = (random % 3) as i64;
                terminal.visible_right = 9 + (random % 5) as i64;
            }
            _ => {
                let id = terminal.add_character("新", Coord::new(3, 3));
                terminal.set_character_visibility(id, true);
                ids.push(id);
            }
        }
        // Several changes may occur between rendered frames.
        if tick % 3 == 0 {
            check(&mut terminal);
        }
    }
    check(&mut terminal);
    terminal.visible_right = 0;
    check(&mut terminal);
    terminal.visible_top = 0;
    check(&mut terminal);
}

#[test]
fn cached_blocks_follow_variable_length_visuals_and_owner_changes() {
    let mut terminal = Terminal::new(
        "abcdefghijklmnopqrstuvwxyzABC",
        TerminalConfig {
            ignore_terminal_dimensions: true,
            ..TerminalConfig::default()
        },
    )
    .unwrap();
    let ids = terminal.input_characters.clone();
    for &id in &ids {
        terminal.set_character_visibility(id, true);
    }
    check(&mut terminal);
    let long_symbol = "λ".repeat(80);
    // Changes before, within, and after long runs of cached blocks, including
    // a partial final block. Each check renders twice and materializes rows.
    for &index in &[0, 12, 28, 3, 4, 24] {
        for symbol in [long_symbol.as_str(), "é", ""] {
            terminal.arena[ids[index].0 as usize]
                .animation
                .set_appearance("x", false, Some(symbol), None);
            check(&mut terminal);
        }
    }
    let upper = terminal.add_character("█", Coord::new(1, 1));
    terminal.set_character_visibility(upper, true);
    terminal.arena[upper.0 as usize].layer = 1;
    check(&mut terminal);
    // An obscured character changes, then becomes visible when the winner's
    // layer drops. The winner selection must use character_id, not arena slot.
    terminal.arena[ids[0].0 as usize]
        .animation
        .set_appearance("x", false, Some("新"), None);
    check(&mut terminal);
    terminal.arena[upper.0 as usize].layer = -1;
    check(&mut terminal);
    terminal.arena[upper.0 as usize].layer = 0;
    terminal.arena[ids[0].0 as usize].character_id = 10000;
    check(&mut terminal);
    terminal.set_character_visibility(ids[0], false);
    check(&mut terminal);
}
