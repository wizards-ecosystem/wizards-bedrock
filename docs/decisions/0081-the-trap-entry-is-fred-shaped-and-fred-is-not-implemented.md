# D-81 The trap entry is FRED-shaped, and FRED is not implemented

Status: Accepted, 2026-08-24.

**Context.** FRED shipped on Panther Lake 2026-01-27 and is default-on in Linux
7.1. It replaces IDT delivery and `IRET` with `ERETS`/`ERETU`, replaces IST
with four reentrant stack levels, makes `SWAPGS` a `#UD` in favour of `LKGS`,
and delivers a uniform frame regardless of event — and it is the mechanism that
finally makes supervisor shadow stacks tractable. `010` §10 checked whether
QEMU emulates it: QEMU 11.0.50 lists `fred`, `lkgs` and `wrmsrns` among its
known CPU features and accepts `-cpu max,fred=on`. That moves FRED from
unverifiable to testable, but not to implemented.

**Decision.** Phase 5 builds the *shape* and not the mechanism:

- **Dispatch from a table on a decoded `TrapCause`**, rather than 256 IDT stubs
  that each know their own vector. The decode is pure arithmetic and goes to
  `libs/trap-cause`.
- An **opaque `TrapContext`** at call sites, in place of raw frame field access.
- **`swapgs` behind `arch::percpu`**, never open-coded.
- **`iretq` behind `arch::trap::return_to_user`.**
- **Trap stacks as an enum**, mapped onto IST today and onto FRED's four levels
  later.

Alongside, three entry-path items that are one-liners now and near-impossible
to retrofit gracefully: **`VERW` on exit-to-user** gated on `MD_CLEAR`, an
**eIBRS MSR write at boot**, and **`ENDBR64` on every indirect target in
`global_asm!`** so the naked switch does not break when kernel IBT is enabled.
KPTI is decided *against*, with `RDCL_NO` recorded as the reason.

**Rationale.** Every item is an improvement on its own terms, which is the test
that separates shaping from speculation. The decoded cause is what an aarch64
port would need, which is a strong signal it is the right abstraction rather
than speculative generality. The opaque context is what D-77 already forced for
the frame. `arch::percpu` has to exist for ring 3 regardless. And a table
dispatch is smaller than 256 stubs.

The three one-liners are shaped by cost asymmetry rather than by likelihood:
`VERW` is one instruction on a return path that is painful to modify once
mature, and eIBRS is one MSR write at boot. KPTI is declined because its cost
scales with syscall rate — under 0.5% below 10k/s/CPU, ~5% at 75k, and PCID cut
it from 2.1% to 0.5% — and this kernel's target hardware reports `RDCL_NO`.

**Alternatives.** Implement FRED now (rejected: the transitions are not proven
emulated, the hardware is one generation old, and the shape captures the design
benefit at a fraction of the cost); keep the 256 stubs and grow into them
(rejected: the entry path is rebuilt exactly once, and this is the phase that
rebuilds it); defer `VERW` and eIBRS to a security pass (rejected on retrofit
cost, which is the only reason they are here).

**Consequences.**

- `libs/trap-cause` is a new host-tested crate, checked over the whole vector
  space including the reserved range.
- The existing `global_asm!` trampoline — the most battle-tested assembly in
  the repo, which Phase 4 declined to convert once already (D-51) — is
  converted here, because the table dispatch is what it feeds.
- Whether `syscall`/`sysret` or a trap gate carries the synchronous call set is
  **not** decided here. `sysret`'s non-canonical `RCX` hazard is a real CVE
  class costing a branch on every return, a trap gate costs more per call, and
  the measurement that would settle it is meaningless under TCG. It is decided
  in slice 5 on safety and revisited on real hardware.
