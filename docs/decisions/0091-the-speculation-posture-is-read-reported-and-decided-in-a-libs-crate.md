# D-91 The speculation posture is read, reported, and decided in a `libs/` crate

Status: Accepted, 2026-08-24.

**Decision.** `libs/x86-cpuid` owns the decode: whether the processor reports
`IA32_ARCH_CAPABILITIES` at all, what its bits mean, whether a boot-time IBRS
write is wanted, and what `IA32_SPEC_CTRL` should be left as. `wost` keeps the
`cpuid` and the two MSR accesses. The posture is printed at boot. D-81's eIBRS
write, `VERW` and KPTI refusal are all statements about what the *processor*
claims, and this is how the kernel asks.

- **Silence is not safety.** A processor with no capability word claims nothing,
  a different value from immunity, not the same `false`. Treating silence as
  `RDCL_NO` would skip a mitigation on every Intel part before 2018 and most
  AMD parts.
- **The MSR is not read unless the feature bit says it exists.** Reading
  `IA32_ARCH_CAPABILITIES` without it is a `#GP`.
- **Enabling IBRS preserves every other bit.** Clearing a mitigation firmware
  enabled would silently weaken the machine.

Without enhanced IBRS, **nothing is written**. Per-entry IBRS is a cost on a
hot path and a decision for a phase that can measure it.

**`VERW` is not implemented, because there is no exit-to-user path yet.** It
arrives in slice 5 with `return_to_user`, gated on `MDS_NO` being *absent*,
which this crate now decodes.

**Rationale.** QEMU's TCG does not emulate `IA32_ARCH_CAPABILITIES`, so on this
development loop the enabling branch cannot run at all. The choice was to ship
an unexercised branch in the privileged crate, or move every decision it makes
where synthetic capability words can drive it: D-30 and D-43's pattern.

**Alternatives.** Keep the decode in `wost` and note the gap (rejected: the
memory-safety TCB is the worst place for an untestable branch). Skip eIBRS until
hardware is available (rejected on D-81's retrofit argument, and the report is
worth having alone). Treat an absent capability word as all-clear (rejected: it
is the majority case and the wrong answer for it).

**Consequences.**

- `libs/x86-cpuid` is where CPUID-derived arithmetic goes from here. Slice 5's
  XSAVE offset table (D-82) joins it under the same requirement: synthetic CPUID
  snapshots, host-tested, `wost` keeping four instructions.
- On QEMU the boot log reads "describes no speculation capabilities", which is
  true.
- **This decision has no machine check.** The gate cannot reach the enabling
  path. Host tests cover synthetic words, and the first bare-metal boot will
  exercise the rest.
