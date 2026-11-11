# D-445 Foundation resource bands follow attributed security ownership

Status: Accepted, 2026-10-26. Amends D-384, advances PW-722.

**Decision.** Recenter only the resource bands whose ownership changed.
Release accepts 10,100,000 to 11,000,000 ELF bytes and 113,700 to 113,950 free
frames. Debug accepts 13,200,000 to 14,500,000 bytes and 112,600 to 112,900 free
frames. Both require exactly eight live heap allocations. The byte-band widths
remain 900,000 and 1,300,000; the frame-band widths remain 250 and 300. All
timing, heap-size, handle, storage, image, counter and unsafe-source limits,
the three boots per profile and the complete self-test verdict remain unchanged.
Runtime allocation capacities and stack bounds are unchanged. Retained mutation
534 must fail the existing two-sided band test. Historical rebuilds and paused
paging observations explain the decision; they do not substitute for
current-source qualification.

**Rationale.** The 294-check kernel exceeded D-384's 281-check bands; at
`43cbd56` it measured 10,527,880 and 13,794,456 ELF bytes, 113,834 to 113,847
and 112,756 to 112,757 free frames, and eight live allocations. Frame use after
initialization rose by seventeen in each profile: D-433's owner-drop witnesses
intentionally retain sixteen frames, and the larger or shifted image needs one
more leaf table (release's second leaf, debug's write-xor-execute direct-map
alias). D-389 retains one 48-byte WZDM table in an existing slab class,
explaining the extra live allocation without another heap frame. Bootloader
placement varies, so this is not an exact census of pre-initialization
allocations. Keeping historical limits rejects reviewed retained state, broader
bands would reduce regression sensitivity, stripping debug information would
change the artifact contract without explaining runtime ownership, and timing
and the self-test verdict are unrelated to the measured change.
