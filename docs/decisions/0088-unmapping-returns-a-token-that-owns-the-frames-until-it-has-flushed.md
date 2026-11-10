# D-88 Unmapping returns a token that owns the frames until it has flushed

Status: Accepted, 2026-08-24.

**Context.** D-86 made mapping into a live space possible and D-87 made every
mapping carry its evidence, but neither could give a frame back: the check D-86
added leaks one on purpose and says so. Unmap closes that, and two rules
recorded much earlier come due with it. One is D-70 rule 9, *unmap must return a
flush token rather than flushing, and must not promise global visibility on
return*. The other is the standing Rust constraint that **leaking is safe**, so
no soundness argument may rest on `Drop` running.

**Decision.**

`AddressSpace::unmap(virtual_base, len) -> Result<Unmapped, MapError>` takes a
mapping away and hands back a token. The token **owns the frames**, and they
leave it only through `flush()` or the `Drop` that does the same work.

That ordering is the soundness property. A CPU may still hold a cached
translation to a frame whose entry was just cleared. Handing that frame to the
allocator before the invalidation would let it be given away while a stale entry
still reaches it. So the frames stay unreachable until the `invlpg`s have run.
`mem::forget` on a token is therefore safe: it costs the frames, not
correctness, which is how the design survives Rust's leak rule.

**Provenance lives in the page entry.** `Flags::OWNED` is bit 9, one of the
three the architecture leaves to software (`AVL`, SDM Vol. 3A ch. 4), so the CPU
ignores it. `map_owned` sets it; `unmap` refuses any page without it. A frame
the allocator handed out must come back, and one the firmware placed must not.
The entry is the only thing every path to an unmap can see. A table beside it
would need keeping in step with every mapping operation, in a kernel where
address spaces are about to multiply.

**What unmap refuses**, each with its own error: a hole (`NotMapped`), a
misaligned or empty range (`Misaligned`), or a large page (`Unsplittable`). So
are a frame that is not the allocator's (`NotOwned`) and frames that are not one
ascending run (`Discontiguous`). The whole range is preflighted before any entry
is cleared, so a refusal leaves the space as it was. That is the rule D-77 gave
`map`, applied to its inverse.

**Large pages are not split.** The direct map is two-megabyte on purpose (D-45),
and `010` §8's measurement confirmed that choice. A split would be the first
step toward losing it. A caller that wants to unmap inside a large page is
asking the wrong question.

**Rationale.** The token shape is what D-70 asked for, and its value is
invisible on one CPU. A caller taking several mappings away pays one
invalidation instead of one each, and a future SMP shootdown has something to
batch. The ownership arrangement *is* visible on one CPU, and that is the half
worth getting right now, because it is the half that would be unsound if
guessed.

Frames inside the token make the design correct; frames returned beside it would
make it correct only *if callers are careful*. D-79 applied the same reasoning
to ring buffers in the other direction: the kernel owns the buffer, because
blocking in `Drop` is unsound when leaking is safe.

**Alternatives.** Flush inside `unmap` and return the frames (rejected: D-70
rule 9, and it forecloses the shootdown batching SMP needs). Return the frames
beside the token and rely on `Drop` to flush (rejected: `mem::forget` then hands
live-reached frames to the allocator, and leaking is safe). Track ownership in a
side table (rejected: it must stay in step with every mapping operation across
every address space, and the entry cannot drift from itself). Split large pages
on demand (rejected on D-45 and `010` §8).

**Consequences.**

- `Flags::OWNED` and `Entry::ABSENT` join `libs/x86-paging`, host-tested: the
  owned bit lies in the `AVL` window, overlaps no hardware flag, and does not
  move the address it decorates.
- `cpu::invalidate_page` is a *safe* function, and the module doc says why. The
  tables are the truth and the TLB caches them, so dropping an entry can only
  force a re-read. The unsound direction is not invalidating.
- The check `a mapping added after the switch is live` gained `OWNED` in its
  expected flags. Its exact flag comparison found the change by failing.
- Two checks, both broken red: releasing the frames before the flush, and
  assuming provenance instead of reading it. Eighty-four named machine checks.
- **D-86's leaked frame is still leaked**, on purpose. That check maps and never
  unmaps, because it is about the mapping being live. It costs four pages for
  the life of a boot.
- Still owed by this slice: the thread stack that arrives and leaves with its
  thread, which turns `STACK_SLOTS` from a cap into a default (D-52), and the
  boot and double-fault stack guards.
