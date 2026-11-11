# D-335 PCI configuration changes consume one exact function authority

Status: Accepted, 2026-09-28. Amends D-168, D-265 and D-329; closes PW-118, PW-125 and PW-129.

**Decision.** One opaque function claim binds its issuing registry, canonical
function identity and live generation. Every command transition consumes a
sublease from that claim and may complete only while the installed command
word still matches. BAR probing derives its count from the verified header and
returns the exact restoration owner whenever hardware state is uncertain;
that owner continues excluding other claimants until an exact retry succeeds.
Neither the pure authority nor its hardware wrapper is `Copy` or `Clone`, and
the command registry and unchecked function-release door remain private.

**Rationale.** D-265 implemented an exclusive claim with retained recovery
and D-329 proved its identities opaque, but closure needed causal evidence
that a command snapshot cannot cross a registry or function, transitions
cannot nest, commit cannot overwrite a changed hardware word, uncertain BAR
restoration keeps exclusion and its retry owner, and every configuration write
requires non-duplicable authority rather than a raw address.

**Evidence.** Retained mutations 749, 750, 757 through 759, 787, 792 and 1,010
through 1,015 each reach their exact witness, panic or contract failure, one
being
`an uncertain bar restore retains exclusion and retries exactly`. A manual
`Clone` on the hardware wrapper and a separate safe duplication door each
compiled, duplicated the embedded authority and failed the exact hostile
contract. D-168 records the verified-header BAR-count break and D-329 the
private-field breaks. PW-536 and PW-537 still own the separate
bus-master/quiescence and physical-range qualifications; PW-201 still owns
native VMware observation.

**Alternatives.** Keeping raw `Address` write doors was rejected because an
address proves neither ownership nor liveness. Dropping an uncertain retry was
rejected because it strands recoverable authority. Releasing the lease after
an unverified restoration was rejected because uncertain hardware state must
remain excluded, not become concurrently mutable.

**Amended by D-343.** The safe-duplication evidence above held for one method
name: the hostile probe called `duplicate`, so a door under any other name
compiled unrefused. D-343 replaces that probe with a syntax inventory of every
door in `pci-config` and `wost`, and mutation 1,013 now fails it.
