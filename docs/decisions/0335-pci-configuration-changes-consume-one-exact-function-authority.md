# D-335 PCI configuration changes consume one exact function authority

Status: Accepted, 2026-09-28. Amends D-168, D-265 and D-329; closes PW-118, PW-125 and PW-129.

**Context.** D-265 implemented an exclusive PCI function claim and retained
recovery, while D-329 proved that the claim identities remained opaque.
Closure still required current causal evidence that a command snapshot cannot
cross a registry or function, command transitions cannot nest, commit cannot
overwrite a changed hardware word, and uncertain BAR restoration neither
releases exclusion nor loses its retry owner. It also required the hardware
wrapper and pure authority to remain non-duplicable and every configuration
write to require that authority rather than a raw address.

**Decision.** One opaque function claim binds its issuing registry, canonical
function identity and live generation. Every command transition consumes a
sublease from that claim and may complete only while the installed command
word still matches. BAR probing derives its count from the verified header and
returns the exact restoration owner whenever hardware state is uncertain;
that owner continues excluding other claimants until an exact retry succeeds.
Neither the pure authority nor its hardware wrapper is `Copy` or `Clone`, and
the command registry and unchecked function-release door remain private.

**Evidence.** On 2026-09-28 retained mutations 749, 750, 757 through 759, 787,
792 and 1,010 through 1,015 were applied individually and restored exactly.
Ignoring registry identity, function identity or live claim membership failed
the exact cross-registry or configuration-cycle witnesses. Reversing the
nested-lease test failed the exclusion witness; omitting the installed-word
comparison reached the exact `a changed command word cannot be committed`
panic. Dropping uncertain retry authority failed
`an uncertain bar restore retains exclusion and retries exactly`; releasing
the probe first let a competing command claim succeed and failed that same
witness.

Deriving `Clone` or `Copy` for the pure authority made the hostile compiler
probe compile and failed its exact authority contract. A manual `Clone`
implementation on the hardware wrapper and a separate safe duplication door
also compiled, each duplicated the embedded authority without weakening the
pure type and failed the exact hostile contract. Publishing the command
registry or unchecked release door let the corresponding parallel-registry or
live-sublease bypass compile and failed the exact contract. D-168 already
records the independently broken verified-header BAR-count witness, and D-329
records the independent private-field breaks for both authority identities.

Exact restoration passed all 78 `pci-config` unit tests and both doctests, the
PCI authority contract, and all 1,412 mutation applicability and named-check
registrations. Restored release QEMU passed 262/262 after claiming both NVMe
functions, enabling memory decode with bus mastering still off, enabling bus
master only after verified controller disable, and starting a controller with
8-entry queues, 133,120 512-byte blocks and four-byte-spaced doorbells.
PW-536 and PW-537 still own the separate bus-master/quiescence and physical-
range qualifications; PW-201 still owns native VMware observation.

**Alternatives.** Keeping raw `Address` write doors was rejected because an
address proves neither ownership nor liveness. Dropping an uncertain retry was
rejected because it strands recoverable authority. Releasing the lease after
an unverified restoration was rejected because uncertain hardware state must
remain excluded, not become concurrently mutable.

**Amended by D-343.** The safe-duplication evidence above held for one method
name: the hostile probe called `duplicate`, so a door under any other name
compiled unrefused. D-343 replaces that probe with a syntax inventory of every
door in `pci-config` and `wost`, and mutation 1,013 now fails it.
