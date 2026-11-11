# D-250 Shared kernel mappings require positive runtime evidence

Status: Accepted, 2026-08-30. Amends D-81 and D-91; verification tracked by PW-534.

**Decision.** A process may be constructed with shared kernel mappings only
for one of two opaque reasons: an affirmative `RDCL_NO` in a described
architectural-capability word, or the exact QEMU TCG base tuple. The latter
requires the CPUID.1 hypervisor bit, maximum hypervisor leaf `0x40000001` and
the twelve-byte `TCGTCGTCG` signature. Missing capability evidence, VMware,
other hypervisors and every near match refuse. Positive `RDCL_NO` remains
sufficient regardless of environment.

The TCG reason is trusted local development configuration, not attestation and
not a hardware security claim. Every repository QEMU launch disables ambient
user configuration, selects `-accel tcg`, and exposes the hypervisor and
`tcg-cpuid` features in both normal and degraded CPU profiles. Interactive
arguments may still select displays, debuggers, devices and diagnostics, but
the runner rejects every CPU, machine, accelerator, global-property or config
file avenue that can replace the fixed profile, including long aliases and
machine-embedded accelerator selection. Legacy `-set` remains permitted
because it addresses an existing named QemuOpts group and cannot replace these
scalar machine, CPU or accelerator selections.

Boot evaluates the policy once and records a distinct admission state before
process construction. An accepted reason is logged; refusal leaves diagnostic
kernel service running but returns `NoSharedKernelIsolation` immediately after
the guarded-stack check and before page tables or any other process resource
exist. Automated QEMU success additionally requires exactly one exact
`qemu_tcg` log record, proving that the guest consumed the policy chosen by the
runner. It does not prove that an untrusted hypervisor told the truth.

**Rationale.** D-81 declined KPTI because the target hardware reports
`RDCL_NO`, but admission never checked the processor actually running the
image, and D-91's distinction between an absent `IA32_ARCH_CAPABILITIES` and an
all-clear word affected only reporting and eIBRS, so ring 3 received the kernel
in its page tables on a silent processor. The pinned QEMU does not expose
`IA32_ARCH_CAPABILITIES`, so a blanket refusal would remove the development
gate, while a generic hypervisor or vendor name is no substitute: QEMU can
use another accelerator, configuration can replace its CPU or machine, and a
hostile monitor can spoof CPUID. The gate needs a narrow statement about the
controlled emulator it started, not a claim that CPUID attests the host.

**Alternatives.** Treating silence as `RDCL_NO` was rejected by D-91 and would
fail open on the processors that most need isolation. Refusing every silent
machine was rejected for the controlled TCG loop because it would remove all
ring-3 machine evidence without improving native-hardware safety. Accepting a
generic QEMU, VMware or hypervisor signature was rejected because those names
do not constrain the execution engine or CPU model. Inferring TCG solely from
the host command was rejected because later arguments and ambient config can
override it and because the guest must demonstrate that it reached the same
decision. KPTI is not silently waived: PW-706 triggers it before supported
native hardware without `RDCL_NO` may launch user programs.

**Evidence and consequences.** Pure host tests refuse an undescribed word, a
described zero word, the historical VMware tuple and every one-bit mutation of
all four QEMU tuple registers, and prove the hypervisor leaf is not read
without the hypervisor bit. Host runner tests pin every launch site, reject all
profile-changing aliases and config avenues, allow unrelated debug arguments,
and require the exact guest record only for a passing verdict. The machine
check injects the production refusal, requires the typed error, no syscall
crossing and an unchanged resource ledger, then proves the accepted state was
restored. Its retained mutation and the current QEMU and VMware observations
remain PW-534; no boot result is invented here.
