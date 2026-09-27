# Incremental Rust renderer

The Rust renderer now follows the cell-ownership and output-cache algorithms in
`asm/engine/render.asm`, instead of clearing and repainting the entire grid on
every frame.

- Each cell retains a linked list of occupants and its winning character.
  Movement and visibility changes unlink/link characters in constant time.
- Cells affected by membership or painter-key changes are queued once per
  frame. After all changes, each queued cell selects the greatest
  `(layer, character_id)` among its occupants. This preserves the Rust ordering
  contract, including character IDs that differ from arena slots.
- Cells retain their winning visual. Unchanged visual identities leave cached
  bytes intact; retained `Rc`s prevent address reuse or in-place edits from
  invalidating that identity check.
- Rows cache their bytes and the byte offsets of four-cell blocks. Changed
  blocks are emitted again; contiguous unchanged blocks are copied as one run
  with rebased offsets. Entire unchanged rows skip rebuilding.
- Changes to clipping bounds or canvas offsets invalidate the grid and caches.
  Both the frame API and the bottom-up `terminal_state` API use the same renderer.

There are still deliberate differences from assembly: Rust scans the visible
characters to observe edits to public fields rather than requiring every caller
to emit change-log records. It returns a contiguous `String`, so it copies cached
rows into that output instead of sending row buffers with `writev`. This change
does not introduce a renderer thread or port the assembly simulation engine.
Caching adds persistent per-character, per-cell, and per-row storage. Retaining
visuals can also require appearance setters to allocate a replacement visual.

No performance claim is made: timing comparisons were stopped because the host
CPU was busy. Future comparisons should build both revisions with
`cargo build --release --no-default-features` (or set `TTFX_ASM=0`), then use
`tools/tests/bench_compare.py` on an idle machine.

`tests/render_incremental.rs` compares cached output against an independent
sort-and-repaint reference across movement, collisions, layer/ID changes,
visibility, clipping/layout changes, added characters, Unicode, long symbols,
and changing byte lengths around cached block boundaries. The existing parity
corpus can also compare frame streams against the previous Rust binary.
