# D-384 The foundation baseline follows reviewed security state, not its historical footprint

Status: Accepted (2026-10-11), amends D-230

**Context.** D-230's kernel-size, free-frame and live-allocation bands described
the 215-check foundation. The completed 281-check kernel now retains the
bounded objects needed for PCI topology ownership, interrupt isolation, VT-d
unit selection and DMA revocation. The release ELF grew from 6,725,512 to
9,610,296 bytes and the debug ELF from 8,648,304 to 12,680,760 bytes. Release
free frames moved from 114,885 to 114,125; debug moved from 114,265–114,266
to 113,119–113,132. Both profiles now retain seven live heap allocations
instead of four. Applying the historical bands therefore refused before the
qualification could sample a kernel whose ordinary correctness gate was green.

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

**Proof.** A diagnostic six-boot run first used deliberately broad temporary
limits. Release observed 6,099,553 to 6,600,862 ns, exactly 114,125 free
frames, heap state 18/1,002/7, 34,679 to 38,986 ns at the trap boundary and
23,828,996 to 35,060,943 bytes/s from storage. Debug observed 37,484,605 to
39,606,770 ns, 113,119 to 113,132 free frames, heap state 18/1,002/7, 147,711
to 200,902 ns and 8,093,033 to 9,871,837 bytes/s. Both images remained exactly
67,108,864 bytes, both handle counts remained zero and the unsafe-source count
remained exactly 381. The temporary limits were then replaced by narrow state
bands and the complete six-boot qualification repeated. The first source-bound
post-commit qualification then caught the two historical timing ceilings
described above; the reviewed timing bands were adjusted and the complete
qualification repeated again.

**Alternatives.** Shrinking or stripping the ELF would make the host artifact
smaller without returning the security-owned runtime state and would discard
the debug information intentionally retained by the release profile. Keeping
the historical frame and allocation numbers would encode a kernel that no
longer exists. Keeping timing ceilings below observed fixed-machine variance
would make normal host scheduling look like a kernel regression. Far broader
permanent limits would make the alarm insensitive to the next large regression.
Moving the task into ordinary CI would add six boots to every commit without
improving its role as a reviewed release qualification.
