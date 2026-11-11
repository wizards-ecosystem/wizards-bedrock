# D-77 The completed-phase audit treats green as a claim to attack

Status: Accepted, 2026-08-24.

**Decision.** A completed phase's green is a claim to attack: an adversarial
pass covers every implemented boundary, and Phases 0 to 4 harden as one
checkpoint:

- The panic proof and lock-recovery operation are crate-private. `wost` enters
  recovery only through a non-returning function-pointer boundary, and kernel
  panic output uses `try_lock`, so no safe caller can recover a lock and
  resume through a live guard.
- Interrupt handlers receive `&TrapFrame`, with private, read-only fields. The
  raw 22-word machine frame has offsets asserted at compile time and its
  *extent* checked by a live same-privilege probe: the frame base plus the
  claimed words must land on the interrupted stack pointer, the half no
  compile-time assertion can see. The assembly aligns every Rust call
  explicitly; `InterruptState` is a single-use, non-transferable restoration
  token.
- Firmware access is an ACPI-only copying capability. RSDP, SDT, pointer-array
  and MADT parsing enforce the caller's buffer and the structure's declared
  budget before exposing a prefix. `Rsdp::parse` reads the identifying bytes
  before the revision, so its error names non-RSDP bytes, not an undefined
  revision. Every silent refusal on the firmware-read path names the address
  it refused, because this narrower authority makes a routing-less boot the
  likely first encounter.
- Mapping preflights the whole range before writing an entry. Activation
  proves physical identity for the running code and stack, image permissions
  across linked and direct aliases, and every stack page and guard in the
  candidate root. Parent tables become user-accessible only when a user leaf
  needs it.
- Device mappings use checked extent arithmetic, reject zero and malformed
  spans, refuse RAM, boot, kernel, ACPI and framebuffer storage, reserve their
  physical interval, and verify every claimed translation. The framebuffer's
  validated geometry is mapped once, WC; a reversible read-modify transition
  changes PAT entry 1. Kernel text is not writable through the direct map.
- Hardware-free bounds live in their `libs/` crates; `libs/mmio-range` owns
  repeated-access alignment, multiplication and end-bound arithmetic.
- A self-test verdict is exactly one well-formed versioned record with the
  expected non-zero count; missing, duplicate and malformed records fail.
  Serial read errors fail, debug-exit must agree with the record, and the
  shipped boot receives a key and must show IRQ-driven input progress.

**Rationale.** Each defect crossed an abstraction boundary whose prose outran
its evidence; narrow capabilities, preflighted arithmetic and
physical-identity checks make those claims executable. No caution from today's
caller makes safe a second mapping with a different cache type, a public panic
token, a mutable return frame, or a parser that yields a valid prefix after a
corrupt tail.

**Alternatives.** Keep the historical APIs with call-site checks (rejected:
another safe caller can omit them); treat the second framebuffer alias as a
performance-only risk (rejected: cache-type conflicts are a correctness
boundary); accept the first valid boot verdict (rejected: trailing or
duplicate records become invisible); postpone everything to Phase 5 (rejected:
user authority would rest on unsound kernel authority).

**Consequences.**

- The boot contract is 79 named machine checks. Every new refusal and positive
  path was weakened once and seen to redden its host or boot check before
  restoration.
- **Corrected the same day.** The first entry-path check recorded the stack
  pointer *after* the alignment mask, so `alignment == 0` was a tautology:
  D-28's failure mode, in the commit claiming every check was broken red. It
  now records where the frame begins and checks its extent.
- D-31, D-34, D-50, D-64 and D-76 are amended where their mechanisms were too
  broad. What they protected stands: terminal kernel faults, IRQ-safe locks,
  WC framebuffer output, bounded firmware input, and a sound `wost` boundary.
- Two authority debts wait for the phase that brings their owner types. Raw
  `AddressSpace::map` can still name a physical address, and port claims prove
  exclusivity but not device entitlement. Phase 5 must replace the first with
  owned user-memory mappings; Phase 6's PCI discovery must narrow the second
  for discovered devices.
- QEMU supplies the automated machine evidence. Phase 3 still lacks its dated
  VMware close record; no later audit may manufacture one retroactively.
