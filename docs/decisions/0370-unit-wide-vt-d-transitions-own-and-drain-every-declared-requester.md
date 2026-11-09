# D-370 Unit-wide VT-d transitions own and drain every declared requester

Status: Accepted (2026-10-09), amends D-357, D-358, D-360 and D-369

**Context.** Deep Codex Security scan
`32245094-b8b7-47d1-b6e7-a52554800772` found that the storage lifecycle owns
one PCI function while `GCMD.TE`, `GCMD.QIE` and `RTADDR` control a complete
DRHD unit. `Plan::read` disabled translation for that unit after fencing only
the selected NVMe function; another endpoint governed by the same DRHD could
therefore continue bus mastering while translation was off. Candidate release
repeated the same unit-wide gap. These are occurrences
`occ_bf1abc446165512e6ba909ab` and
`occ_e11dd8768ab5b7fe79e4e235`.

The same review found a stronger owner-loss path. Establishment returned only
the register window for every programming error. A root-table command or
translation-enable write can take effect even when the following read or poll
fails, so the old result could drop tables which hardware might already be
walking. Release retained the tables on a failed disable, but described the
unit as still on even though the write might instead have succeeded and only
its observation failed. The self-test-only off/on door exercised the same
unit-wide transition while merely borrowing an idle selected controller.

An endpoint match is not exclusivity evidence: one DRHD may carry several
endpoint scopes, or combine one with `INCLUDE_PCI_ALL`. Conversely, blindly
fencing functions found by the ordinary topology walk is not complete enough
for a security authority: bridge ranges, unresolved paths, other segments,
hot-pluggable catch-all requesters and a bounded claim registry can all leave
an unowned requester behind.

**Decision.** `acpi-tables` exposes one allocation-free live PCI requester
inventory for the exact governing DRHD. The selected function must be reached
by an endpoint or resolved PCI sub-hierarchy scope, never only by
`INCLUDE_PCI_ALL`. Endpoint paths resolve exactly. A sub-hierarchy contributes
its bridge requester and scans every bus, device and function number in its
validated secondary-to-subordinate range without trusting multifunction bits;
each live result is resolved through `unit_for` again, so an endpoint or a
hierarchy whose exact live bridge path proves it nested may assign a requester
to another unit, while overlapping sibling hierarchies are ambiguous and
refused rather than ordered by bus number. Every selected function must occur
in the result, and the result must fit the fixed function-authority capacity.
No unit may make the proof scan more than sixteen PCI requester scopes;
duplicate declarations do not create duplicate owners but still spend this
hostile-work budget. Catch-all, unresolved, unsupported requester scopes,
ambiguous and oversized inventories are refused before any unit register is
mapped or written. I/O APIC and HPET interrupt scopes do not name PCI DMA
requesters and remain outside this inventory.

Before the first unit-wide write, the kernel holds the selected function's
existing command owner and freshly verifies BME clear. It exclusively claims
every other declared requester, clears BME with exact readback through a new
function-bound command transaction, and keeps each command and function owner
attached to the installed DRHD. The safe restoration target preserves every
captured command bit except BME. A partial fence acquisition is unwound only
through those owners; any uncertain command or claim return is retained and
halts selection.

The remapping unit must advertise both read and write DMA draining. After all
BME sources are closed, takeover first quiesces translation and queued
invalidation, then completes a global IOTLB invalidation with both Drain Reads
and Drain Writes before publishing the new root. The explicit drain is required
even when firmware already left TES clear: merely observing that state says
nothing about requests issued before the BME fences. On release, Intel's TE
contract makes observed TES-off completion the corresponding drain boundary.
A unit without both capabilities is refused before a command write. Thus
endpoint and bounded sub-hierarchy scopes may share a unit without an
untranslated in-flight interval, while unbounded catch-all coverage remains
deliberately unsupported rather than guessed.

The kernel has no PCI hot-plug or post-enumeration configuration service.
Admission therefore treats the boot topology as static, and the unit owner
holds exclusive function and command authority for every requester it found
for the entire transition and installed lifetime. A future hot-plug path must
extend this admission boundary before it may publish or enable a new requester.

VT-d establishment becomes an ownership-carrying state machine. Capability
and status preflight, table allocation and all fallible preparation precede
the first unit mutation. Once quiescence or root publication is attempted,
every failure returns the complete `Vtd` owner with its registers and all table
frames intact. The kernel stores that owner together with every requester
fence and halts selection. Release likewise keeps the complete installed owner
for any uncertain disable; diagnostics say the unit state is uncertain rather
than claiming translation stayed on. Only observed TES-off completion may
separate the register mapping from the tables and release requester claims,
whose final command words still exclude BME.

The self-test-only translation toggle is removed. The production `domain`
door continues to read live TES before minting its linear proof, and a source
oracle owns that invariant without creating a weaker unit-transition API.
The large domain remains in the global installed slot. Opaque per-requester
function and command authorities live in one fixed global vault whose compact
linear token is inseparable from the installed unit; losing that token retains
the safe claims. The temporary bounded inventory stays allocation-free, and a
borrowing sibling isolation stage keeps its construction out of the
owner-moving claim frame. The 32 KiB candidate stack is neither enlarged nor
its two-page margin weakened.

**Proof required.** Pure DMAR tests must cover two endpoint scopes on one unit,
duplicates, endpoint plus include-all, hierarchy-root and downstream requester
expansion, selected functions behind a hierarchy, more-specific foreign-unit
exclusion, overlapping sibling-hierarchy refusal, bounded duplicate-scope work,
unresolved paths, valid long-path precedence, unsupported requester scope, foreign governance,
segment mismatch and fixed inventory overflow. Pure PCI tests must start with
inherited sibling BME, cover every
write/read/rollback/retry position, bind the transaction to the exact function
generation, and prove its final restoration differs only by BME. VT-d tests
and source contracts must require both drain capabilities, encode both IOTLB
drain request bits, place their completed drain and every later unit write
after the requester fence boundary, preserve the complete owner after every
post-write uncertainty, and leave no test-only bypass.

The ordinary machine must show both NVMe endpoint scopes owned by one DRHD,
retire the first candidate, retain its BME-off fence through the unit handoff,
and select the second without losing isolation. Release, debug, shipped and
no-IOMMU boots must retain D-369's queue, baseline and degraded-storage
witnesses. Every added check needs an independently observed retained red
mutation before the ordinary gate.

**Evidence.** On 2026-10-09 the pure suites passed 77 `acpi-tables`, 84
`pci-config`, 34 `vtd` and 23 DMA-contract tests. The complete privileged host
gate passed 724 tests with 10 deliberately ignored, every one of the 1,953
public API items passed the isolated compiler audit, every host and kernel
feature graph passed Clippy, and the supply-chain gate accounted for 72
registry crates, three artifacts, four runtime inputs and all three vendored
payloads. The 1,673-entry mutation registry remains applicable and has a
permanent identity; retained mutations 1,665 through 1,679 independently red
requester completeness and deduplication, hierarchy range/root/ambiguity,
catch-all and unsupported-scope refusal, inherited and drifting BME, both
drain capabilities and bits, release ordering, drain placement, hostile scope
work, and the isolation-stage stack margin. In particular, mutation 1,679
added one 4 KiB stage frame and failed only the two-page stack witness at
275/276 with 8,104 bytes free.

Ordinary release and debug QEMU boots each passed 276/276 checks. The debug
stack witness used 23,792 of 32,768 bytes and retained 8,976 bytes of headroom
with a 22,192-byte deepest leaf. The ordinary machine inventoried nine live
requesters on one DRHD, retired candidate 1 while retaining its fence, and
selected candidate 2 through the same unit. The shipped boot repeated that
handoff, mounted storage, entered ring 3, advanced the keyboard IRQ count from
24 to 30 and settled. Both degraded boots settled: the no-PCI/no-x2APIC image
kept optional protection failure diagnostic-only, and the no-DMAR image
retired both candidates with BME off, refused unisolated storage and continued
without it. A fresh independent post-patch review found and drove fixes for an
already-disabled-unit drain omission, numerically ordered sibling hierarchies
and duplicate-scope proof-work amplification; it found no remaining
post-write owner-loss path.

**2026-10-16 validation amendment.** Mutation 1,665 still names the production
defect in this decision: skipping the first governed requester must fail
`every shared vtd requester was fenced across controller fallback`. Its two
inconclusive observations exhausted the ordinary 60-second boot bound while
earlier deliberate red paths ran; they did not contradict the late witness.
Ordinary check and panic mutations now receive an explicit 180-second
observation budget, while a timeout mutation retains its own exact registered
deadline. Source tests bind both the deadline selection and its forwarding into
QEMU. This is mutation-harness implementation only; mutation 1,665 must still
be observed red in the replacement complete sweep before qualification closes.

**Alternatives.** Treating `Coverage::Endpoint` as singleton evidence misses
sibling endpoint scopes. Fencing only the ordinary topology walk leaves bridge
requesters, hidden function numbers and unresolved requester IDs unproved.
Clearing BME without a hardware drain does
not settle already-issued transactions. Updating a live root to block siblings
is not a substitute: VT-d permits live SRTP replacement only when old and new
tables produce the same translation results for in-flight requests. A strict
single-endpoint policy closes the finding but needlessly rejects the qualified
two-controller QEMU layout, whose governing hierarchy currently expands to
nine live requesters; the bounded twelve-owner inventory provides the same
complete authority while preserving that path. Retrying after an
uncertain unit write, freeing unpublished-looking tables after SRTP, or
asserting that a timeout means TES stayed on all discard evidence the hardware
did not supply.
