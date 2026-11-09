# D-77 The completed-phase audit treats green as a claim to attack

Status: Accepted (2026-08-24)

**Context.** A fresh adversarial pass covered every implemented Phase 0–4
boundary: pure arithmetic, boot handoff, interrupt entry and return, physical
memory, page-table construction and activation, device mapping, firmware
parsing, panic recovery, scheduler state, input protocols, and the host gate.
The baseline was green. It nevertheless admitted safe ways to invalidate live
guards, rewrite an interrupt return frame, read arbitrary described physical
memory, alias one device with conflicting cache types, map RAM as MMIO, and
partially mutate page tables before rejecting malformed input. The gate also
accepted ambiguous verdict streams and did not prove that the shipped keyboard
path progressed by interrupt.

**Decision.** The completed phases are hardened as one checkpoint:

- The panic proof and lock-recovery operation are crate-private. `wost` enters
  recovery only through a non-returning function-pointer boundary, and kernel
  panic output uses `try_lock`; no safe caller can recover a lock and then
  resume through a live guard.
- Interrupt handlers receive `&TrapFrame`, whose fields are private and
  read-only. The raw 22-word machine frame has its offsets asserted at compile
  time and its *extent* checked by a live same-privilege probe — the frame base
  plus the words the struct claims must land on the interrupted stack pointer,
  which is the half no compile-time assertion can see. The assembly aligns
  every Rust call explicitly, and `InterruptState` is a single-use,
  non-transferable restoration token.
- Firmware access is an ACPI-only copying capability. RSDP, SDT, pointer-array
  and MADT parsing each enforce both the caller's buffer and the structure's
  declared budget before exposing a prefix.
- Page mapping preflights the whole range before writing an entry. Activation
  proves physical identity for the running code and stack, image permissions
  across both linked and direct aliases, and every stack page and guard in the
  candidate root. Parent tables become user-accessible only when a user leaf
  requires it.
- Device mappings use checked extent arithmetic, reject zero and malformed
  spans, refuse RAM, boot, kernel, ACPI and framebuffer storage, reserve their
  physical interval, and verify every claimed translation. The framebuffer's
  validated geometry is mapped once with WC; PAT entry 1 is changed by a
  reversible read-modify transition. Kernel text is not writable through the
  direct map.
- Hardware-free bounds moved into the relevant `libs/` crates. The pass fixed
  frame-run search at region edges, pixel stride and format validation, slab
  placement overflow, private stack-layout invariants, huge-page PAT encoding,
  canonical mapping ranges, EEVDF's half-range ordering, PS/2 prefix state,
  and the local APIC's legal spurious-vector floor. `libs/mmio-range` now owns
  repeated-access alignment, multiplication and end-bound arithmetic.
- A self-test verdict is exactly one well-formed versioned record with the
  expected non-zero count; missing, duplicate and malformed records fail.
  Serial read errors fail, debug-exit must agree with the record, and the
  shipped boot receives a key and must show IRQ-driven input progress.

**Rationale.** Each defect crossed an abstraction boundary whose prose was
stronger than its evidence. Narrow capabilities, preflighted arithmetic and
physical-identity checks make those claims executable. A second mapping with a
different cache type, a public panic token, a mutable return frame, or a parser
that yields a valid prefix after a corrupt tail cannot be made safe by asking
today's caller to be careful.

**Alternatives.** Keep the historical APIs and add call-site checks (rejected:
another safe caller can omit them); treat the second framebuffer alias as a
performance-only risk (rejected: cache-type conflicts are a correctness
boundary); accept the first valid boot verdict (rejected: trailing or duplicate
records then become invisible); postpone all changes to Phase 5 (rejected: the
next phase would build user authority on unsound kernel authority).

**Consequences.**

- The current boot contract is 79 named machine checks. Every new refusal and
  every new positive path was deliberately weakened once and observed to turn
  the corresponding host or boot check red before restoration.
- **Corrected the same day.** The entry-path check as first written recorded
  the stack pointer *after* the alignment mask, so `alignment == 0` was a
  tautology — D-28's failure mode, in the commit that claimed every check had
  been broken red. It now records where the frame begins and checks its extent,
  and that claim was earned by shortening the declared frame and watching the
  boot report 78 of 79. Two smaller corrections rode along: `Rsdp::parse` reads
  the signature before the revision, so bytes that are not an RSDP are named by
  the field that says so rather than by an undefined revision; and every silent
  refusal on the firmware-read path now says which address it would not read,
  because the narrower authority of this entry makes a routing-less boot the
  likely way it is first met.
- D-31, D-34, D-50, D-64 and D-76 are amended where their original mechanisms
  were too broad. The decisions they protected — terminal kernel faults,
  IRQ-safe locks, WC framebuffer output, bounded firmware input, and a sound
  `wost` boundary — stand.
- Two authority debts intentionally remain for the phase that introduces the
  needed owner types: raw `AddressSpace::map` can still name a physical address,
  and port claims prove exclusivity but not device entitlement. Phase 5 must
  replace the first with owned user-memory mappings; PCI discovery in Phase 6
  must narrow the second for dynamically discovered devices.
- QEMU supplies the automated machine evidence. Phase 3 still lacks its dated
  VMware close record, and no later audit may manufacture one retroactively.
