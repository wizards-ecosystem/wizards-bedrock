# D-88 Unmapping returns a token that owns the frames until it has flushed

Status: Accepted, 2026-08-24.

**Context.** D-86 made mapping into a live space possible and D-87 made every
mapping carry its evidence, but neither could give a frame back: the check D-86
added leaks one on purpose and says so. Unmap is what closes that, and it is
also where two rules recorded much earlier come due — D-70 rule 9, *unmap must
return a flush token rather than flushing, and must not promise global
visibility on return*, and the standing Rust constraint that **leaking is safe**,
so no soundness argument may rest on `Drop` running.

**Decision.**

`AddressSpace::unmap(virtual_base, len) -> Result<Unmapped, MapError>` takes a
mapping away and hands back a token. The token **owns the frames**, and they
leave it only through `flush()` or the `Drop` that does the same work.

That ordering is the soundness property, not an ergonomic one. A CPU may still
hold a cached translation to a frame whose entry has just been cleared; handing
that frame to the allocator before the invalidation would let it be given away
while a stale entry still reaches it. So the frames are unreachable until the
`invlpg`s have run. `mem::forget` on a token is therefore safe: it costs the
frames and not correctness, which is exactly the property that makes the design
survive Rust's leak rule.

**Provenance lives in the page entry.** `Flags::OWNED` is bit 9, one of the
three the architecture leaves to software (`AVL`, SDM Vol. 3A ch. 4), so the CPU
ignores it. `map_owned` sets it; `unmap` refuses any page without it. A frame
the allocator handed out must come back and one the firmware placed must not,
and the entry is the only thing every path to an unmap can see — a table beside
it would have to be kept in step with every mapping operation, in a kernel where
address spaces are about to multiply.

**What unmap refuses**, each with its own error rather than a shared one: a
hole (`NotMapped`), a misaligned or empty range (`Misaligned`), a large page
(`Unsplittable`), a mapping whose frame is not the allocator's (`NotOwned`),
and frames that are not one ascending run (`Discontiguous`). The whole range is
preflighted before any entry is cleared, so a refusal leaves the space exactly
as it was — the rule D-77 gave `map`, applied to its inverse.

**Large pages are not split.** The direct map is two-megabyte on purpose (D-45),
`010` §8's measurement confirmed that choice, and offering a split here would be
the first step toward losing it. A caller that wants to unmap inside a large
page is asking the wrong question.

**Rationale.** The token shape is what D-70 asked for, and its value is not
visible on one CPU: a caller taking several mappings away pays one invalidation
instead of one each, and when the SMP shootdown exists it has something to
batch. What *is* visible on one CPU is the ownership arrangement, and that is
the half worth getting right now, because it is the half that would be unsound
if guessed.

Putting the frames inside the token rather than returning them alongside it is
the difference between a design that is correct and one that is correct *if
callers are careful*. The same reasoning D-79 applied to ring buffers — the
kernel owns the buffer, because blocking in `Drop` is unsound when leaking is
safe — applies here in the other direction.

**Alternatives.** Flush inside `unmap` and return the frames (rejected: D-70
rule 9, and it forecloses the shootdown batching that is the whole point on
SMP); return the frames beside the token and rely on `Drop` to flush (rejected:
`mem::forget` then hands live-reached frames to the allocator, and leaking is
safe); track ownership in a side table (rejected: it must be kept in step with
every mapping operation across every address space, where the entry cannot go
out of step with itself); split large pages on demand (rejected on D-45 and
`010` §8).

**Consequences.**

- `Flags::OWNED` and `Entry::ABSENT` join `libs/x86-paging`, host-tested: the
  owned bit sits in the `AVL` window, overlaps no hardware flag, and does not
  move the address it decorates.
- `cpu::invalidate_page` is a *safe* function, and the module doc says why: the
  tables are the truth and the TLB is a cache of them, so dropping an entry can
  only force a re-read. The unsound direction is not invalidating.
- The check `a mapping added after the switch is live` gained `OWNED` in its
  expected flags, and found the change by failing — an exact flag comparison
  doing its job.
- Two checks, both broken red: releasing the frames before the flush, and
  assuming provenance instead of reading it. Eighty-four named machine checks.
- **D-86's leaked frame is still leaked**, deliberately: that check maps and
  never unmaps, because what it is about is that the mapping is live. The
  frames it costs are four pages for the life of a boot.
- Still owed by this slice: the thread stack that arrives with its thread and
  leaves with it, which is what turns `STACK_SLOTS` from a cap into a default
  (D-52), and the boot and double-fault stack guards.
