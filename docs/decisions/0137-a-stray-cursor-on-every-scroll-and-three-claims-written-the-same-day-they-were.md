# D-137 A stray cursor on every scroll, and three claims written the same day they were false

Status: Accepted, 2026-08-25.

**Context.** The prose audit over Phases 0–5 was asked to find comments and
documents that say what the code does not, on the standing that a false comment
is a defect rather than untidiness — the next reader spends their scepticism
elsewhere. It returned a long list. Most of it is numbers and stale
cross-references, handled elsewhere. Four items are this entry's, and the first
is a live rendering bug.

**The console carried its cursor up the screen on every scroll.**
`set_cursor`'s own doc says the cursor is "drawn only while the console is
idle — every write hides it first and shows it again afterwards, which is what
keeps it from being scrolled up the screen as a stray mark."

`write_text` does exactly that. `write_line` did this:

```rust
pub(crate) fn write_line(&mut self, text: &str) {
    self.write_text(text);   // ...ends with set_cursor(true)
    self.newline();          // ...and this is what scrolls
}
```

The line break is **outside** the bracket. So `newline` — the call that
triggers the scroll — ran with the cursor visible, and `copy_rows` moved the
underline up with everything else. Every log line in the kernel goes through
`write_line`, so this happened on every scroll, on every boot, in front of
whoever was watching.

**Three claims that were false the day they were written, two of them mine.**

- `vector.rs` — a commit spliced `SYSCALL` into the *middle* of `KEYBOARD`'s
  doc block, so the ring-3 syscall gate's rustdoc summary read "The keyboard
  controller's line, routed through the I/O APIC", and `KEYBOARD` had no doc at
  all. `missing_docs` cannot see it: both are `pub(crate)`.
- `README.md` — "a check requires this paragraph to name the same set" of
  tasks. No such check exists. The real one compares the *parser* to the *help
  text* and never reads this file, and the sets differ: `clean` and
  `verify-vendor` are named nowhere in it. Written in `4f11e49`, this session.
- `port.rs` — "`pic::silence` is the one caller that does this today", of the
  trap where a failed second claim strands the first permanently.
  `i8042::Controller::new` `?`-chains `DATA` and `CONTROL`: a refused `CONTROL`
  strands `DATA` and the keyboard controller can never be built again on that
  boot. Written in `2402763`, this session, as part of D-129 — which was itself
  an entry about a comment that overclaimed.
- Two ADR citations point at the wrong decision: `paging.rs`'s range-check rule
  cites D-84 (the ring model) where the rule is D-83's, and the README's
  degraded boot cites D-83 where it is D-90's. `xtask/src/decisions.rs` checks
  that a citation *resolves*, not that it resolves to the right thing.

**Decision.** The line break moves inside the bracket, and the two writers
share a `put` that does no cursor handling — so neither can put the cursor back
before the other is done. The claims are corrected to what is true, including
naming the second stranding caller.

**A machine check drives it, and building one required making the console
readable at all.** `Framebuffer::pixel_at` reads through the shadow, so it
costs no uncached device read; `dev::console_probe` returns the pixel, the
packed foreground and the line height under one guard, because three calls
would let a log line land between them. The check writes a screenful of blank
lines — a region whose only possible foreground mark is a cursor, since a glyph
would put marks there legitimately — and requires the row a scroll moved
through to be clear of the foreground colour. Restoring the old ordering turns
it red.

**It runs last, and that is load-bearing.** Placed among its neighbours it
scrolls the console out from under every other console check and turned the
editing check red. A self-test check must be free of side effects on the
running kernel; this one cannot be, so it goes where there is nothing left to
disturb.

**Rationale.** The cursor bug is the answer to a question worth asking about
this whole sweep: what does a *rendering* defect cost, if the kernel is
otherwise sound? It cost nothing but the appearance of the thing — and it
survived five phases, a hundred and thirty-seven checks and six audits, because
**nothing in this kernel could read a pixel back.** `005` even carried
`read_pixel` as a naming example for an accessor that did not exist. The
console was the one subsystem whose output no check could observe, and the one
subsystem with a visible defect. That is not a coincidence.

The three false claims share a shape worth stating: each was written *beside*
the thing it described, by someone looking at it, and each was wrong about a
second instance somewhere else. `pic::silence` really is a caller that strands;
the sentence failed by saying *the one*. A claim of uniqueness is a claim about
the whole tree, made from one file.

**Consequences.**

- `write_line` and `write_text` share `put`. The cursor bracket is each
  writer's own.
- `Framebuffer::pixel_at` is a new `wost` export, and the first way anything
  has had to ask what is on the screen.
- 138 checks. The new one runs last, with a comment saying why it must.
- `port.rs` names both stranding callers. Neither is fixed here: a claim is
  permanent by design, so neither *can* guard against it, and the atomic
  multi-range claim that would let them is owed.
- The two miscitations are corrected. Nothing catches this class — a citation
  that resolves to the wrong decision reads exactly like one that resolves to
  the right one.

**Left owed.** An atomic multi-range port claim, so a caller wanting two ports
either gets both or strands neither. Both callers want it and neither can be
made correct without it.

A citation that names the wrong decision is not detectable by machine, and
there are two hundred citations. This entry corrects the two the audit found by
reading; it does not claim there are no others.
