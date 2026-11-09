# D-177 A check that names an address names a machine

Status: Accepted (2026-08-25)

**Context.** With D-176's controller in place the VMware boot went from
`passed=117 failed=63` to `passed=172 failed=9`. Five of the nine were the
keystroke checks in a headless run with nobody to press `a`. The other four were
real, and they had one thing in common:

```
a boot mapping cannot claim ownership          FAILED
a user space cannot reach the kernel half      FAILED
a user mapping cannot widen a supervisor table FAILED
a differing remap is refused                   FAILED
```

Those four, and no others, call `BootRegion::describe(PAGE_SIZE, PAGE_SIZE)`.
`describe` answers `None` unless the boot memory map covers the range **with
memory**, and VMware's firmware does not describe physical `0x1000` that way.
QEMU's does.

**So none of the four failed.** Each fell out of its setup's `else` arm and
returned `false` — the same value the harness gets when the property is
genuinely violated. Four checks that never reached the code they are about,
reported identically to four security properties coming apart. It took
elimination across ten checks to tell which.

**Decision.** Two things.

**A caller that needs a described range asks for one.**
`MemoryMap::first_memory_run(len)` returns the first page-aligned run of that
many bytes the map covers with memory, never physical zero. The four probes cut
their pages from it. On QEMU the answer is the page they already named, so
nothing about the existing boots changes; on any other firmware the checks now
run instead of evaporating.

**A precondition that can fail gets its own line.** *the map offers a run to
probe* is checked before the four that depend on it, so a machine that cannot
supply one says so once rather than reddening four unrelated properties.

**The rule:**

> **A check that hardcodes an input hardcodes the machine that supplied it.**
> D-165 says a check must reach the code it is about; this is how it stops —
> not by being deleted, but by having its setup quietly answer `None` somewhere
> the author never booted. Where the setup can fail, the failure is a separate
> check, because "the property is false" and "the check did not run" are
> different findings and the harness renders them the same colour.

**Rationale.** This is D-155's rule (choose inputs where the answers differ)
arriving from the other side. `0x1000` was not chosen to make anything differ;
it was chosen because it was there, and being there is a fact about QEMU. The
four properties involved are among the most load-bearing in the kernel — they
are the ones standing between safe code and a second owner of a frame the
kernel is running on — and on the machine D-18 makes this project answer to,
none of them had ever been evaluated.

That is the reason D-18 exists, stated more sharply than the project has yet
had cause to state it: **a second machine does not re-run the checks, it
discovers which of them were about the first machine.**

**Consequences.**

- `wost::boot::MemoryMap` gains `first_memory_run`; `wost::mm` re-exports it
  through the map's lock. No unsafe, no new lock.
- 182 machine checks. The new one is broken by mutation 66.
- The four probes are unchanged on QEMU by construction, which is what makes
  this a fix rather than a rewrite: the gate's result before and after is the
  same because the run it finds is the page that was named.
- Phase 6's VMware boot is closer but still not clean: the keystroke checks
  need a human at the VM, and the clock's refusal is intermittent — it
  calibrated on the boot that found all this, having refused on the one before.
  D-176's diagnosis is what will name it when it recurs.
