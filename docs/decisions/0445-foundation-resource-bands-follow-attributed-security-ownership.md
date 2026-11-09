# D-445 Foundation resource bands follow attributed security ownership

Status: Accepted (2026-10-26); amends D-384, advances PW-722

**Context.** The 294-check kernel exceeds D-384's 281-check resource bands.
Three release and three debug observations at `43cbd56` measured ELF sizes of
10,527,880 and 13,794,456 bytes, free-frame ranges of 113,834 to 113,847 and
112,756 to 112,757, and heap state 18/1,002/8 in both profiles. The preceding
checkpoint already exceeded the release ELF ceiling; post-initialization
frame consumption is unchanged by that cleanup.

Exact-source D-384 diagnostic rebuilds passed all 281 checks in both profiles.
Post-initialization frame consumption rose from 1,236 to 1,253 in release and
1,237 to 1,254 in debug. D-433's owner-drop witnesses intentionally retain
sixteen frames. Read-only walks of live historical and later page tables
account for the remaining frame: release's larger kernel image requires a
second leaf table; debug's shifted image requires another leaf table for its
write-xor-execute direct-map alias. D-389 retains one 48-byte WZDM table in an
existing slab class, explaining the extra live allocation without another heap
frame. The larger ELF reservation and loaded image also reduce the incoming
usable-memory map; bootloader placement varies, so this is not an exact census
of pre-initialization allocations.

**Decision.** Recenter only the resource bands whose ownership changed.
Release accepts 10,100,000 to 11,000,000 ELF bytes and 113,700 to 113,950 free
frames. Debug accepts 13,200,000 to 14,500,000 bytes and 112,600 to 112,900 free
frames. Both require exactly eight live heap allocations. The byte-band widths
remain 900,000 and 1,300,000; the frame-band widths remain 250 and 300. All
timing, heap-size, handle, storage, image, counter and unsafe-source limits,
the three boots per profile and the complete self-test verdict remain unchanged.
Runtime allocation capacities and stack bounds are unchanged.

**Proof required.** The existing two-sided band test must pass, retained
mutation 534 must fail it, and the restored test must pass. The complete
six-boot baseline and ordinary gate must pass with these permanent limits.
Historical rebuilds and paused paging observations explain the decision;
they do not substitute for current-source qualification.

**Alternatives.** Keeping historical limits rejects reviewed retained state.
Broader bands would reduce regression sensitivity. Stripping debug information
would change the artifact contract without explaining runtime ownership.
Relaxing timing or accepting a failed self-test is unrelated to the measured
resource changes and is rejected.
