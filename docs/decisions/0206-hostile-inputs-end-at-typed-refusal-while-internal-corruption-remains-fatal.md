# D-206 Hostile inputs end at typed refusal, while internal corruption remains fatal

Status: Accepted, 2026-08-28. Amends D-41, D-115 and D-170; closes PW-400.

**Decision.** Target boundary crates deny production `unwrap`, `expect` and
direct `panic` shortcuts with compiler lints; tests may use them to state their
fixtures. `kernel` and `wost` deny production `unwrap` and `expect` across the
whole crate. A repository check keeps those declarations on the explicit
boundary inventory. This is deliberately not a blanket ban on assertions:
trap, scheduler and ring assertions name internal state corruption after safe
recovery has become impossible, and treating those as malformed-input
refusals would let the machine continue with two ownership ledgers disagreeing.

`PortSpan` remains an opaque, consuming ownership lease. Its unused accessors
and read/write operations are removed; the first real I/O-BAR driver must add
the smallest checked interface its registers force. FAT construction validates
that the complete reserved-plus-table extent precedes `total_sectors` and that
the root cluster lies in the resulting complete-cluster count before a `Bpb`
can exist. File-offset walking splits the offset into a bounded cluster quotient
and in-cluster remainder, so no accumulated byte counter can wrap.

**Rationale.** Malformed ring, image, firmware, filesystem and device data must
not panic either release or debug kernels. Dynamic PCI I/O windows are ownership
leases, not register interfaces, and keeping an unused safe hardware door
solely to preserve its documented panic violates D-41's no-speculative-API rule.
`fat32::Bpb::parse` had accepted a table extent at or beyond the volume's total
sector count, so `Bpb::clusters` panicked in debug and wrapped in release onto
off-volume clusters, and a maximum file offset overflowed `u32` while walking a
corrupt chain. A parser returning `Result` while indexing or subtracting before
the refusal is not total. Establishing the geometry once gives every later
method a type invariant and avoids scattering defensive saturation that would
turn corruption into plausible addresses. Quotient/remainder expresses the
file position directly and costs no more device reads than repeated addition.
Compiler lints make the easy regression fail at its source; corpus tests remain
necessary for arithmetic and indexing whose safety comes from prior checks.

**Alternatives.** Saturating the FAT subtraction was rejected because an
impossible layout would mount as an empty or enormous volume instead of being
named malformed. Clamping the root cluster was rejected because it changes
which directory firmware supplied. Returning errors from the unused dynamic
port accessors was rejected because no caller exists to define or exercise the
right error contract. Banning every assertion was rejected because a failed
irreversible scheduler or completion transition has no safe state to return
to; the boundary proof is that hostile values cannot select those indices.

**Consequences.** The historical four target-side `expect` calls are gone:
D-191 removed the two reserved-slot calls, and this decision removes the two
unused dynamic-port calls together with their entire speculative access API.
Compiler lints now guard `kernel`, `wost` and the explicit nine-crate
boundary inventory; inserting a production `expect` made Clippy fail at that
line, while removing a lint declaration or restoring a `PortSpan` accessor
made the repository check fail.

Three FAT host witnesses cover an impossible table extent, a root cluster
outside the complete data area and a maximum file offset on a corrupt chain.
The existing adversarial ACPI, ELF, PCI, NVMe, ring, path, keyboard and APIC
corpora continue to exercise the other checked constructors. Five persistent
host mutations retain the new source and arithmetic witnesses; no machine
protocol or unsafe operation was added.
