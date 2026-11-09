# D-166 The time is a page, not a call

Status: Accepted, 2026-08-25. Amended by D-181.

**Context.** D-152 settled the shape of a published clock and D-149 put it in
this phase, before a filesystem exists to want it, because a call that becomes
a load later is two doors to one operation for ever. `sys_clock_get` was OWED
over `libs/timebase` (D-114): the mechanism was complete and the door was
missing.

**Decision.** One page, mapped read-only into every program at a fixed address,
with two regions.

**The constants region has no sequence counter and needs none.** Version, page
size, counter rate — written once, before the page can be mapped anywhere, and
never again. Zircon's arrangement, and it removes the most error-prone reads
from the protocol entirely.

**The clock region carries Linux's seqlock**, both barriers written out even
though on x86 the store-store one is a compiler barrier and no instruction —
because an aarch64 port should not have to rediscover that it is not free
everywhere.

**D-181 corrects the claimed reason this clock has no periodic update.** Linux
rewrites a published clock for adjustable or idle-stopping sources and for NTP.
D-57 deliberately does *not* veto calibration when CPUID omits invariant TSC:
two believable, agreeing measurement windows are accepted and the missing bit
is warned about. The conversion and origin remain fixed for the supported
uptime, so the page and `time::now_ns` compute the same value, but on a machine
whose rate later changes both clocks are only as steady as that counter. The
mode permits revocation; suspend, NTP adjustment and an instability detector
remain absent.

**The base is split, and the obvious form is wrong.** `base_ns << shift`
overflows a `u64` after about eight seconds of uptime at a 1 GHz counter's
shift, so "store the nanoseconds pre-shifted" cannot be taken literally. What
is stored is whole nanoseconds beside a carried *fraction* of one, always below
`1 << shift`. Linux splits it the same way. The fraction is not decoration: a
clock updated at a kilohertz that dropped it would fall a microsecond behind
every second.

**The revocation gate is why the Uptime fallback stays.** `mode` is the reader's
permission to use the page at all, and D-152 requires the fallback to be always
reachable — so the ring's `Uptime` opcode is not decoration either, and the
check that revokes the clock asks the ring for the time on the way past.

**Rationale, and the three things that were wrong first.**

*The page exists even when the clock does not.* The ABI says there is a page at
`WIZ_TIME_ADDRESS`, and a program that reads it and takes a page fault has been
lied to. So the page is built at boot whatever the clock did, `mode` is
`MODE_NONE` on a machine that never calibrated, and a spawn that cannot show it
is refused rather than quietly given a space with a hole in it.

*The preemption precondition was measured in the wrong place.* D-152 asks that
"the writer is not preemptible" be established rather than assumed, and the
first counter recorded **calls** made with the interrupt flag set. The check
built on it failed immediately — correctly, because a kernel thread calling
`publish` with interrupts on is fine: the guard clears the flag before the
counter goes odd and restores it after it goes even. What matters is the
*window*, so that is what is counted now.

*The odd marker has no witness on one CPU.* Deleting the write that marks the
window open left every check on this page green. Nothing else runs between the
two increments — the guard clears the interrupt flag — so there is no observer.
This is the class D-154 handles by building the seam and checking what can be
checked: the writer now reads its own marker back and counts a failure, so a
writer that stops announcing itself is caught by the only observer that exists.
A second CPU would give a better one.

**Consequences.**

- `libs/timebase` gains `resume`, `advance`, `from_parts` and `Seq`, all
  host-tested, four deliberate breaks. The drift test needed two attempts: the
  first chose a step of 3,192,000 ticks at 3.192 GHz, which is exactly a
  millisecond, so the conversion had no fraction to lose and dropping the carry
  left it green — D-155's rule, hit again by the person who wrote it down.
- Six machine checks, six breaks, all red. The page-and-kernel-clock check is a
  **sandwich rather than a tolerance**: the page's reading is taken between two
  calls and must lie between them, so a multiplier or shift off by one fails
  immediately and no slack hides it.
- `sys_clock_get` moves from OWED to MAPS and is now a load. The sysdep tally
  is 5/10/1 where it was 4/11/1. This is the historical one-axis transition;
  D-181 records the checked monotonic reader and missing fallback/realtime
  semantics as PARTIAL/PARTIAL.
- `WIZ_CALL_CLOCK_GET` is **deleted** from the ABI, and `WIZ_CALL_RING_WAIT`
  takes 3. At this decision the speculative owed calls renumbered, which cost
  nothing because none was implemented. D-181 later deleted those unimplemented
  call numbers instead of preserving a fictitious future ABI.
- The C header gains the time page's address, version, modes, offsets and
  layout, and the cross-check compares **every one of them** against
  `timepage.rs`, including each field offset. A `mode` read from the wrong
  place is a clock that reads as revoked, or worse as usable when it is not.
- One more entry on D-46's KASLR ledger: `TIME` at `0x0000_1000_0000_0000`.
  It costs nothing while no two programs share a table (D-113), and it is what
  makes the address a `#define` rather than a lookup.
