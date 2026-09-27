//! Incremental cell ownership and cached output, following asm/engine/render.asm.
//! Characters remain publicly mutable, so each frame observes visible characters
//! once. Only changed membership/ordering reselects a cell's winner; only changed
//! visuals rebuild output blocks. No simulation state is cached by raw pointer.

use std::rc::Rc;

use super::animation::CharacterVisual;
use super::character::{CharId, EffectCharacter};

const NONE: usize = usize::MAX;
const BLOCK: usize = 4;

#[derive(Clone)]
struct Slot {
    cell: usize,
    prev: usize,
    next: usize,
    key: (i64, u32),
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            cell: NONE,
            prev: NONE,
            next: NONE,
            key: (0, 0),
        }
    }
}

#[derive(Default)]
struct Cell {
    head: usize,
    owner: usize,
    pending: bool,
    visual: Option<Rc<CharacterVisual>>,
}

#[derive(Default)]
struct Row {
    bytes: Vec<u8>,
    scratch: Vec<u8>,
    offsets: Vec<usize>,
    next_offsets: Vec<usize>,
    dirty: Vec<bool>,
    changed: bool,
}

#[derive(Default)]
pub(super) struct Renderer {
    geometry: Option<[i64; 6]>,
    width: usize,
    slots: Vec<Slot>,
    cells: Vec<Cell>,
    pending: Vec<usize>,
    rows: Vec<Row>,
}

impl Renderer {
    fn mark_pending(&mut self, cell: usize) {
        if !self.cells[cell].pending {
            self.cells[cell].pending = true;
            self.pending.push(cell);
        }
    }

    pub fn remove(&mut self, slot: usize) {
        let Some(state) = self.slots.get(slot) else {
            return;
        };
        let (cell, prev, next) = (state.cell, state.prev, state.next);
        if cell == NONE {
            return;
        }
        if prev == NONE {
            self.cells[cell].head = next;
        } else {
            self.slots[prev].next = next;
        }
        if next != NONE {
            self.slots[next].prev = prev;
        }
        self.slots[slot].cell = NONE;
        self.mark_pending(cell);
    }

    fn set_visual(&mut self, cell: usize, visual: Option<&Rc<CharacterVisual>>) {
        let same = match (&self.cells[cell].visual, visual) {
            (None, None) => true,
            (Some(old), Some(new)) => Rc::ptr_eq(old, new),
            _ => false,
        };
        if !same {
            // Retaining an Rc makes in-place visual edits use copy-on-write,
            // and prevents allocator address reuse from hiding a change.
            self.cells[cell].visual = visual.cloned();
            let row = &mut self.rows[cell / self.width];
            row.dirty[(cell % self.width) / BLOCK] = true;
            row.changed = true;
        }
    }

    pub fn update(&mut self, arena: &[EffectCharacter], visible: &[CharId], geometry: [i64; 6]) {
        let [left, right, bottom, top, column_offset, row_offset] = geometry;
        let width = right.max(0) as usize;
        let height = top.max(0) as usize;
        if self.geometry != Some(geometry) {
            let count = width
                .checked_mul(height)
                .expect("terminal canvas is too large");
            self.geometry = Some(geometry);
            self.width = width;
            self.slots.clear();
            self.pending.clear();
            self.cells = (0..count)
                .map(|_| Cell {
                    head: NONE,
                    owner: NONE,
                    ..Cell::default()
                })
                .collect();
            self.rows = (0..height)
                .map(|_| Row {
                    dirty: vec![true; width.div_ceil(BLOCK)],
                    changed: true,
                    ..Row::default()
                })
                .collect();
        }
        self.slots.resize(arena.len(), Slot::default());
        for &CharId(id) in visible {
            let slot = id as usize;
            let ch = &arena[slot];
            let row = ch.motion.current_coord.row + row_offset;
            let column = ch.motion.current_coord.column + column_offset;
            let cell = if bottom <= row
                && row <= top
                && left <= column
                && column <= right
                && row >= 1
                && column >= 1
            {
                (row - 1) as usize * width + (column - 1) as usize
            } else {
                NONE
            };
            let key = (ch.layer, ch.character_id);
            if self.slots[slot].cell != cell {
                self.remove(slot);
                if cell != NONE {
                    let head = self.cells[cell].head;
                    self.slots[slot].cell = cell;
                    self.slots[slot].prev = NONE;
                    self.slots[slot].next = head;
                    if head != NONE {
                        self.slots[head].prev = slot;
                    }
                    self.cells[cell].head = slot;
                    self.mark_pending(cell);
                }
            }
            if self.slots[slot].key != key {
                self.slots[slot].key = key;
                if cell != NONE {
                    self.mark_pending(cell);
                }
            }
            if cell != NONE && self.cells[cell].owner == slot {
                self.set_visual(cell, Some(&ch.animation.current_character_visual));
            }
        }
        // Resolve collisions only after all moves, so a crowded cell is scanned
        // once even if many occupants leave it in the same frame.
        while let Some(cell) = self.pending.pop() {
            self.cells[cell].pending = false;
            let mut winner = NONE;
            let mut slot = self.cells[cell].head;
            while slot != NONE {
                if winner == NONE || self.slots[slot].key > self.slots[winner].key {
                    winner = slot;
                }
                slot = self.slots[slot].next;
            }
            self.cells[cell].owner = winner;
            let visual =
                (winner != NONE).then(|| &arena[winner].animation.current_character_visual);
            self.set_visual(cell, visual);
        }
        for (index, row) in self.rows.iter_mut().enumerate() {
            if !row.changed {
                continue;
            }
            row.scratch.clear();
            row.next_offsets.clear();
            let mut block = 0;
            while block < row.dirty.len() {
                if row.dirty[block] {
                    row.next_offsets.push(row.scratch.len());
                    let start = block * BLOCK;
                    for cell in &self.cells
                        [index * width + start..index * width + (start + BLOCK).min(width)]
                    {
                        if let Some(visual) = &cell.visual {
                            visual.formatted_symbol.append_to(&mut row.scratch);
                        } else {
                            row.scratch.push(b' ');
                        }
                    }
                    row.dirty[block] = false;
                    block += 1;
                } else {
                    // Copy a whole clean run, rebasing its block boundaries for
                    // any preceding visual whose encoded byte length changed.
                    let start = block;
                    let output_start = row.scratch.len();
                    while block < row.dirty.len() && !row.dirty[block] {
                        row.next_offsets
                            .push(output_start + row.offsets[block] - row.offsets[start]);
                        block += 1;
                    }
                    row.scratch
                        .extend_from_slice(&row.bytes[row.offsets[start]..row.offsets[block]]);
                }
            }
            row.next_offsets.push(row.scratch.len());
            std::mem::swap(&mut row.bytes, &mut row.scratch);
            std::mem::swap(&mut row.offsets, &mut row.next_offsets);
            row.changed = false;
        }
    }

    pub fn rows(&self) -> impl DoubleEndedIterator<Item = &str> + ExactSizeIterator {
        self.rows.iter().map(|row| {
            // SAFETY: rows contain only whole UTF-8 symbols or cached blocks
            // whose boundaries were recorded between those symbols.
            unsafe { std::str::from_utf8_unchecked(&row.bytes) }
        })
    }
}
