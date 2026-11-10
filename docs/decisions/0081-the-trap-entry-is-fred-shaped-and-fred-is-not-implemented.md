# D-81 The trap entry is FRED-shaped, and FRED is not implemented

Status: Accepted, 2026-08-24.

**Context.** FRED shipped on Panther Lake 2026-01-27 and is default-on in Linux
7.1. It replaces IDT delivery and `IRET` with `ERETS`/`ERETU` and IST with four
reentrant stack levels. It makes `SWAPGS` a `#UD` in favour of `LKGS` and
delivers a uniform frame for every event. It is also the mechanism that finally
makes supervisor shadow stacks tractable. `010` §10 checked QEMU: 11.0.50 lists
`fred`, `lkgs` and `wrmsrns` among its known CPU features and accepts
`-cpu max,fred=on`. That moves FRED from unverifiable to testable, but not to
implemented.

**Decision.** Phase 5 builds the *shape* and not the mechanism:

- **Dispatch from a table on a decoded `TrapCause`**, not 256 IDT stubs that
  each know their own vector. The decode is pure arithmetic and goes to
  `libs/trap-cause`.
- An **opaque `TrapContext`** at call sites, in place of raw frame field access.
- **`swapgs` behind `arch::percpu`**, never open-coded.
- **`iretq` behind `arch::trap::return_to_user`.**
- **Trap stacks as an enum**, mapped onto IST today and onto FRED's four levels
  later.

Three entry-path items are one-liners now and near-impossible to retrofit
gracefully: **`VERW` on exit-to-user** gated on `MD_CLEAR`, and an **eIBRS MSR
write at boot**. The third is **`ENDBR64` on every indirect target in
`global_asm!`**, so the naked switch survives kernel IBT. KPTI is decided
*against*, with `RDCL_NO` recorded as the reason.

**Rationale.** Each item pays on its own terms, which separates shaping from
speculation. An aarch64 port would need the decoded cause, a strong sign it is
the right abstraction. D-77 already forced the opaque context for the frame.
`arch::percpu` has to exist for ring 3 regardless. A table dispatch is smaller
than 256 stubs.

Cost asymmetry, not likelihood, shapes the three one-liners. `VERW` is one
instruction on a return path that is painful to modify once mature, and eIBRS is
one MSR write at boot. KPTI is declined for two reasons. Its cost scales with
syscall rate: under 0.5% below 10k/s/CPU, ~5% at 75k, and PCID cut it from 2.1%
to 0.5%. And this kernel's target hardware reports `RDCL_NO`.

**Alternatives.** Implement FRED now (rejected: the transitions are not proven
emulated, the hardware is one generation old, and the shape gives the design
benefit cheaply). Keep the 256 stubs and grow into them (rejected: the entry
path is rebuilt once, in this phase). Defer `VERW` and eIBRS to a security pass
(rejected on retrofit cost, the only reason they are here).

**Consequences.**

- `libs/trap-cause` is a new host-tested crate, checked over the whole vector
  space including the reserved range.
- The existing `global_asm!` trampoline is converted here, because the table
  dispatch is what it feeds. It is the most tested assembly in the repo, and
  Phase 4 declined to convert it once already (D-51).
- This decision does **not** choose whether `syscall`/`sysret` or a trap gate
  carries the synchronous call set. `sysret`'s non-canonical `RCX` hazard is a
  real CVE class costing a branch on every return, and a trap gate costs more
  per call. The deciding measurement is meaningless under TCG. Slice 5 decides
  on safety, and hardware revisits it.
