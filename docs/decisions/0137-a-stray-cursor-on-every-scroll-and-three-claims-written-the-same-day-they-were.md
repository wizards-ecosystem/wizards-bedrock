# D-137 A stray cursor on every scroll, and three claims written the same day they were false

Status: Accepted, 2026-08-25.

**Decision.** `write_line`'s line break moves inside the cursor bracket, and
`write_line` and `write_text` share a `put` that does no cursor handling, so
neither can put the cursor back before the other is done. `set_cursor`'s doc
states the rule: the cursor is drawn only while the console is idle, which
keeps it from being scrolled up the screen as a stray mark; `newline` scrolled
with it visible.

`Framebuffer::pixel_at` reads through the shadow, so it costs no uncached
device read, and is the first way anything can ask what is on the screen.
`dev::console_probe` returns the pixel, the packed foreground and the line
height under one guard, because three calls would let a log line land between
them. The check writes a screenful of blank lines, a region whose only possible
foreground mark is a cursor, and requires the row a scroll moved through to be
clear of the foreground colour. **It runs last**, with a comment saying why: it
scrolls the console out from under every other console check, and a self-test
check that cannot be free of side effects goes where there is nothing left to
disturb.

False claims are corrected: `SYSCALL`'s doc no longer sits inside `KEYBOARD`'s,
the README no longer claims a check reads its task list, and `port.rs` names
both stranding callers (`pic::silence` and `i8042::Controller::new`). Two
miscitations are corrected: `paging.rs`'s range-check rule cites D-84 where the
rule is D-83's, and the README's degraded boot cites D-83 where it is D-90's.

**Rationale.** A false comment is a defect rather than untidiness. The cursor
bug survived five phases because **nothing in this kernel could read a pixel
back**, and the console was the one subsystem whose output no check could
observe. Each false claim was written beside the thing it described and was
wrong about a second instance elsewhere: a claim of uniqueness is a claim about
the whole tree, made from one file.

**Consequences.** `xtask/src/decisions.rs` checks that a citation *resolves*,
not that it resolves to the right thing: a citation that resolves to the wrong
decision reads exactly like one that resolves to the right one.

**Left owed.** An atomic multi-range port claim, so a caller wanting two ports
either gets both or strands neither. Both callers want it and neither can be
made correct without it.

A citation that names the wrong decision is not detectable by machine, and
there are two hundred citations. This entry corrects the two the audit found by
reading; it does not claim there are no others.
