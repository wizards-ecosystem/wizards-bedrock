# D-384 The foundation baseline follows reviewed security state, not its historical footprint

Status: Accepted, 2026-10-11. Amends D-230.

**Decision.** Keep D-230's measurement procedure and every storage, image,
handle, heap-size, counter and unsafe-source limit unchanged. Replace the
values whose owned state changed: release accepts a 9,200,000 to
10,100,000 byte ELF, 114,000 to 114,250 free frames and exactly seven live
allocations; debug accepts 12,100,000 to 13,400,000 bytes, 113,000 to 113,300
free frames and exactly seven live allocations. Host scheduling also moved a
post-commit release sample to 10,089,424 ns and the preceding ordinary-CI debug
boot to 45,031,550 ns, just outside the historical 9 and 45 ms ceilings. The
reviewed upper timing bounds are therefore 15 ms for release and 70 ms for
debug; their 3.5 and 20 ms lower bounds remain unchanged. These remain
two-sided regression alarms with margin around reviewed observations, not
permissions for unbounded growth.

**Rationale.** D-230's bands described the 215-check foundation; the 281-check
kernel retains the bounded objects needed for PCI topology ownership, interrupt
isolation, VT-d unit selection and DMA revocation, so the historical bands
refused before the qualification could sample a kernel whose correctness gate
was green. Shrinking or stripping the ELF would not return that runtime state
and would discard the debug information the release profile retains; timing
ceilings below observed fixed-machine variance would make host scheduling look
like a kernel regression; far broader limits would miss the next large
regression; and running the task in ordinary CI would add six boots to every
commit without improving its role as a reviewed release qualification.
