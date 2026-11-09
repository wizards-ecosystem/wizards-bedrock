# D-201 Ring three opens only with enforced non-execution and a progressing clock

Status: Accepted (2026-08-28), amends D-12, D-32, D-57, D-96, D-164 and D-166; advances PW-301 and closes PW-302

**Context.** Page-table construction marks data, stacks and shared pages
non-executable, but `EFER.NXE` was enabled only when `CPUID` offered it and
process construction never asked whether the read-back succeeded. On such a
processor the intended W^X boundary was not a property the CPU enforced.
Separately, failed APIC-clock calibration logged that nothing would preempt
and then still launched ring 3. An infinite program could own the processor,
Timer could reserve a completion which no interrupt would ever settle, and
Uptime could report only zero. The first Wizzy runtime is the concrete caller
which must know whether execution and waiting exist before it starts.

**Decision — the minimum ring-3 contract is NX plus monotonic progress.** The
kernel may cross into ring 3 only after the enabled-protection read-back says
the no-execute bit is active and the monotonic/APIC clock is calibrated. The
existing guarded-boot-stack prerequisite remains independent and mandatory.
Process construction checks all three before allocating an address space, and
returns a typed refusal naming the missing prerequisite.

WP remains architectural and unconditionally enabled and read back. SMEP,
SMAP and UMIP remain enabled whenever `CPUID` offers them and remain part of
the preferred security baseline, but absence of those three is a supported
diagnostic fallback: the page-table user/supervisor boundary and checked copy
door still exist. NX is different because it is the mechanism which makes
every non-code mapping non-executable. A machine without it may continue far
enough to diagnose itself where its inherited mappings permit, but it is not a
supported userspace target and receives no weaker ring-3 mode.

Timer admission asks the same live calibration predicate before reserving a
pending slot. Without it the operation completes immediately with
`STATUS_UNAVAILABLE`; it is never recorded as in flight. The time page may
still exist as `MODE_NONE`, and Uptime still exists for the post-launch
revocation design PW-303 owns, but a boot which never calibrated cannot launch
a program merely so that both fallbacks can answer zero forever.

**Rationale.** Fail-closed admission is smaller and more truthful than a
second scheduler clock. A PIT interrupt path would need routing, tick policy,
conversion and independent validation; PW-303 must cost those against CPUID
frequency leaves and another clocksource rather than smuggle them in as a
fallback. Keeping the kernel prompt and hardware-free namespaces alive still
makes a refused machine diagnosable without granting unbounded execution.

**Alternatives.** Treating NX as optional and omitting `NO_EXECUTE` from page
entries was rejected because it changes W^X into W-or-X-by-convention exactly
where D-12 requires hardware enforcement. Launching only programs which are
expected to yield was rejected because the kernel cannot verify that property
of arbitrary machine code, including the future compiler artifact. Admitting
Timer and reporting failure later was rejected because admission already
consumed the terminal reservation and promised progress. Using an iteration
bound as preemption was rejected because code which never traps has no loop
the kernel can count.

**Consequences.** Two release-QEMU checks inject the missing prerequisites at
the same predicates production uses, observe typed pre-allocation refusals, no
syscall crossing and exact resource ledgers, and prove a clockless Timer posts
one immediate unavailable completion with nothing pending. Bypassing the NX,
user-clock and Timer-clock gates separately reddened only their named checks;
all three have persistent mutations. The degraded `qemu64` boot reports
mandatory `wp nx`, omits optional SMEP/SMAP/UMIP, and settles after the exact
clock-based ring-3 refusal. The complete gate now carries 208 machine checks
and 139 mutations. PW-301 is `REQUIRED`: the current phase-close VMware round
must show NX, calibration and successful user launch together. PW-303 still
owns clocksource qualification and post-boot revocation; this decision does
not claim those solved.
