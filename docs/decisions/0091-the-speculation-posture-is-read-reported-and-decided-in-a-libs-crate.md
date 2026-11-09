# D-91 The speculation posture is read, reported, and decided in a `libs/` crate

Status: Accepted, 2026-08-24.

**Context.** D-81 listed two entry-path items as one-liners worth taking now
because they are near-impossible to retrofit: `VERW` on exit-to-user, and an
eIBRS MSR write at boot. It also declined KPTI, recording `RDCL_NO` as the
reason. All three are statements about what the *processor* claims, and this
kernel had no way to ask.

**Decision.** `libs/x86-cpuid` owns the decode: whether the processor reports
`IA32_ARCH_CAPABILITIES` at all, what its bits mean, whether a boot-time IBRS
write is wanted, and what `IA32_SPEC_CTRL` should be left as. `wost` keeps the
`cpuid` and the two MSR accesses. The posture is printed at boot.

Three things the decode is careful about:

- **Silence is not safety.** A processor with no capability word is saying
  nothing, not saying it is immune, and the two are different values rather
  than the same `false`. Every Intel part before 2018 and most AMD parts are
  silent, which is exactly the population where treating silence as `RDCL_NO`
  would skip a mitigation.
- **The MSR is not read unless the feature bit says it exists.** Reading
  `IA32_ARCH_CAPABILITIES` without it is a `#GP`, so this is the one property
  here whose failure is a crash rather than a wrong answer.
- **Enabling IBRS preserves every other bit.** Firmware may have enabled a
  mitigation of its own, and clearing it would be the kernel silently weakening
  the machine it booted on.

Without enhanced IBRS, **nothing is written**. The alternative is per-entry
IBRS, which is a real cost on a hot path and a decision for a phase that can
measure it — `010` §8 is explicit that nothing here is measurable on the current
dev loop, because QEMU/TCG timings are meaningless.

**`VERW` is not implemented, because there is no exit-to-user path yet.** D-81
paired the two, but one is a boot-time write and the other is an instruction on
a return path this kernel does not have until ring 3. It arrives in slice 5 with
`return_to_user`, gated on `MDS_NO` being *absent* — which this crate now
decodes.

**Rationale, and the reason the split is not ceremony here.** QEMU's TCG does
not emulate `IA32_ARCH_CAPABILITIES`. It was asked directly:
`-cpu max,+arch-capabilities` and `-cpu Skylake-Server-v5,+arch-capabilities,+rdctl-no,+ibrs-all`
both boot with the feature bit clear, so the kernel reports "describes no
speculation capabilities" and the IBRS write is never reached. **On this
development loop that branch cannot run at all.**

So the choice was between shipping an unexercised branch in the privileged
crate, or moving every decision it makes somewhere synthetic capability words
can drive it. That is D-30 and D-43's pattern, and `010` §8 asks for exactly it
for the XSAVE table on the same grounds. Six host tests now cover the paths a
boot here never takes.

**Alternatives.** Keep the decode in `wost` and note the gap (rejected: an
untestable branch inside the memory-safety TCB is the worst place to put one);
skip eIBRS until real hardware (rejected on D-81's retrofit argument, and the
report is worth having on its own); treat an absent capability word as
all-clear (rejected: it is the majority case and the wrong answer for it).

**Consequences.**

- `libs/x86-cpuid` is where CPUID-derived arithmetic goes from here. Slice 5's
  XSAVE offset table (D-82) joins it rather than starting a second crate, which
  is the same requirement — synthetic CPUID snapshots, host-tested, `wost`
  keeping four instructions.
- The boot log gains a line naming what the machine claims. On QEMU it reads
  "describes no speculation capabilities", which is true and is the point.
- **This is a decision with no machine check, and that is recorded rather than
  hidden.** The gate cannot reach the enabling path. What it does have is six
  host tests over synthetic words, and the first real hardware boot is what
  will exercise the rest — which is another entry on the list `010` §8 closes
  with: *acquire a netboot-able bare-metal machine.*
