# D-123 Four ways safe code reached undefined behaviour, and the road back that was not the only one

Status: Accepted, 2026-08-24.

**Context.** Two adversarial audits over Phases 0–5, held to D-105's two rules.
D-122 took the critical finding; these are the rest of the high ones.

### An eight-byte frame shift, through three doors

The entry stubs push a synthetic zero error code exactly where the CPU does
not. `apic::send_self_interrupt` asserted only `vector >= 16`, so
`send_self_interrupt(17)` — a safe call — delivered an interrupt on a vector
the CPU pushes an error code for, into a stub that pushes a second one. The
frame is eight bytes out for the rest of its life: the tail's `add rsp, 16;
iretq` pops `CS` as `RIP`, and the ring-3 test reads the wrong slot.

An APIC delivery pushes no error code whatever the vector *means*, which is
what makes the vector's meaning and the frame's shape disagree.
`set_timer_one_shot` and `interrupt::init`'s `user_gate` are the same door
twice more — the last one ring-3-callable. All three refuse
`exception::pushes_error_code` now; the predicate already existed.

### `reclaim` freed the tables a bound thread would be switched onto

The only guard was "are these tables in `CR3` *right now*", which a caller
reaches by not being the thread in question. `prepare` loads a slot's root
unconditionally, so freeing a bound space's tables is a fault deferred to
whenever that thread is next scheduled — worse than an immediate one for being
harder to attribute.

Whether anything still names a space is a property of the **task table**, which
`AddressSpace` cannot see. So it takes the answer: `task::release_address_space`
looks, clears the binding from the dead slots naming it, and mints a `Released`.
Guessing with a `CR3` comparison was the shape of the bug.

Two shapes changed along the way, and both were the audit's finding one level
down. `reclaim` **borrows** rather than consumes — a refusal that consumed the
thing it declined to act on leaves a caller unable to try again and leaks the
tables it was protecting. And on success the space is *emptied* rather than
dropped: `is_reclaimed` is true and `may_reach` refuses, which is what replaces
consuming it. Returning the space in an `Err` was the first attempt and clippy
was right to refuse a thousand-byte error variant.

`address_space_of` now reports only a **live** slot's tables. A dead slot's are
nobody's; reporting the last root it had is an answer about the past.

### `cpu::enable_extended_state` was a `pub` front door to the state D-115 fixed

It clears `CR0.EM` and sets `CR4.OSFXSR`, so SSE becomes legal at CPL 3. D-115
fixed the ordering *inside* `xstate::init`; any safe caller could reach the
same state from outside, where `FORM` stays zero and nothing saves the register
file across a switch. `pub(super)` — the ordering argument is about one call
site, and a public door made it an argument about nothing.

### `unmap` was a second road back, and reclamation took it

`USegment`'s doc said `reclaim` is the only road out of untyped memory and that
it zeroes. `unmap` checked `OWNED` and never `USER`, so it handed a program's
pages over as a plain `Segment` — and `user::reclaim` takes exactly that road,
dropping its token rather than flushing it. A dead program's pages went back to
the allocator **holding its bytes**.

Nothing disclosed anything, because every consumer happens to zero on the way
in. That is the honest limit and also the point: the mechanism named as the
guarantee was not the one doing the work, and defence in depth was carrying a
property its owner claimed.

`Unmapped` carries `was_user` now — any page ring 3 could reach makes the whole
run untyped, because a run is handed back as one thing. `flush` refuses such a
range, `take_user` gives a `USegment`, and `Drop` clears before the frames go
back.

**The clearing on drop could not be checked, and I said the mutation tool
covered it.** It did not: removing the clear left all 130 checks green and
`cargo xtask mutate` reported the mutation as a **survivor** — which is what the
tool is for, applied to a claim in a comment I had just written. Those frames
go to the allocator, and reading them again needs it to hand back the same
ones, which it does not promise. So the clear is *counted*, and the tally is
what makes the line testable. A second survivor, "a bound space is not
reclaimed", was the same thing: a real property with nothing constructing the
case. Both have checks now, and all fourteen mutations are caught.

**Consequences.** 131 checks. `a_user_page_comes_back_cleared` follows one
handle the whole way — map, unmap, `take_user`, `reclaim`, read — rather than
asking the allocator for the frame back, because a conjunct conditional on
reuse is one skipped on most boots.

Left owed from these audits: the `libs/` mutation survivors (`x86-cpuid`'s four
SDM constants, `handle-space`'s three identity asserts, `acpi-tables`'
`Sdt::LEN`, and five more), `xtask/src/elf.rs`'s missing `e_machine` and
`e_phentsize` refusals, the gate's two enumeration holes, the ABI cross-check's
19 unchecked constants, thirteen checks that cannot fail, and a list of doc
comments. Each is named in the audits and none is a soundness hole.
