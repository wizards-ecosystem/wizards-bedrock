# D-389 VT-d stays default-deny across topology discovery and controller fallback

Status: Accepted (2026-10-11), amends D-358, D-370 through D-372 and D-388

**Context.** The unit-wide requester inventory expanded a DMAR sub-hierarchy
only through functions reported present by one sealed PCI snapshot. A function
could answer absent when its BDF was visited and become bus-master capable
afterward. The same incomplete inventory then appeared to prove both that every
requester was BME-fenced and that the unit could safely clear translation while
changing owners. `TopologyPresence::Empty` also admitted interrupts without any
installed unit. Thus a mutable endpoint could turn a discovery observation into
an unrestricted-DMA interval during initial storage takeover, candidate parking
or fallback. Re-reading presence cannot prove absence, and a bridge's BME bit is
not an architected fence for every requester below it.

**Decision.** The sole segment-zero DRHD becomes one boot-lifetime kernel owner
immediately after PCI configuration access is reserved and before the first
topology read, but only when segment zero contains no RMRR and static platform
authority proves whole-segment coverage. That authority is either the sole
DRHD's `INCLUDE_PCI_ALL` flag or one unique, checksummed revision-1 `WZDM`
table whose segment and register base exactly corroborate the unique DMAR's
sole RMRR-free DRHD. Neither form reads requester presence. Its root
initially contains no present context, its interrupt table contains no present
entry, and both translation and interrupt remapping must read back enabled. A
scoped and unattested, missing, ambiguous, RMRR-bearing or incapable unit
leaves PCI storage and maskable interrupts closed even when the later topology
snapshot is empty.
The installed owner, not endpoint presence, is the authority for that boundary.

Translation is never disabled after this early establishment. Read-only
preflight, repeated after table allocation and immediately before the first unit
write, requires firmware TE, QIE, IRE and CFI all already inactive. Active TE is
refused instead of attempting a live SRTP to a non-identical root; active queued
invalidation or interrupt remapping is refused instead of guessing that
firmware completed their architectural quiescence protocols. From that inactive
state the kernel masks the fault-event source, publishes and invalidates its
empty DMA and interrupt roots, enables IRE and TE, and completes a final global
read-and-write drain after TE is live. Parking atomically removes the selected
bus root, globally invalidates and drains older requests before clearing table
memory, leaving the empty root and both protection facilities live. Retargeting
prepares one selected context behind an absent root entry, publishes it, and
invalidates while translation stays on. Any failure after hardware publication
retains the entire unit in a permanent quarantine state which neither candidate
retargeting nor interrupt admission may revive, even when hardware readbacks
still resemble an active boundary. The sealed requester inventory and function claims
remain defence-in-depth BME fences and candidate-governance checks, but no
inventory result authorizes a translation-off transition. Firmware RMRRs remain
unsupported, so neither early takeover nor live retarget accepts
firmware-selected mappings.

Upstream QEMU 9.2 emits a presence-derived scoped DMAR for this fixed machine,
which cannot prove the static whole-segment authority required before PCI is
read. The qualification harness therefore supplies a separate checksummed
revision-1 `WZDM` table that attests complete coverage by the fixed profile's
unit at `0xfed90000`. The kernel accepts it only beside one unique DMAR whose
sole segment-zero DRHD has that exact register base and no RMRR; a missing,
duplicate, malformed or mismatched table refuses early ownership. The `WZDM`
bytes are trusted test-platform authority, not a production inference, and the
later sealed PCI snapshot must still inventory all nine observed requesters
within its fixed capacity. Because QEMU's Windows `-acpitable` parser splits
drive-letter paths, the harness runs it from the already verified private
directory and passes only the reserved file name. Interactive arguments cannot
append a competing ACPI table. A firmware environment which leaves any
remapping facility active now fails closed rather than being modified; fresh
VMware qualification must therefore demonstrate the supported inactive
handoff. This decision does not claim that firmware which entered the kernel
with translation disabled had protected DMA before the earliest kernel
takeover; that distinct pre-takeover trust boundary remains subject to its own
release finding.

**Proof required.** Host tests must bind the early-owner call before topology
snapshotting, derive and corroborate the sole RMRR-free unit's register base
without a presence callback, require unique DMAR and `WZDM` tables when the
standard catch-all flag is absent, require a real installed owner for both empty and non-empty
sealed topologies, order the final DMA drain after TE enable, and prove parking
and retargeting contain no translation-disable call. Pure VT-d tests must refuse
each inherited TE/QIE/IRE/CFI facility independently. A live machine check must
observe the same kernel root, enabled translation and deny-all interrupt table
after the qualified controller fallback. Persistent mutations 1,552, 1,553,
1,675, 1,742 and 1,803 through 1,821 independently admit inherited state, omit
the final drain, disable translation while parking, move takeover after the
snapshot, accept no owner, admit live SRTP, mistake a scoped unit for complete
coverage, ignore an RMRR, weaken or bypass the qualified coverage attestation,
permit an ACPI override, drop the repeated preflight, pass an unparseable
Windows table path, revive an uncertain post-enable owner, ignore the attested
register base, accept duplicate DMAR or `WZDM` tables, or detach temporary
attestation cleanup from its reviewed construction boundary, or restore the
retired catch-all refusal as dead public surface; each
must make its named proof red. Release, debug
and degraded boots and the complete CI gate must pass after restoration.

**Alternatives.** Rejecting every sub-hierarchy would unnecessarily narrow
firmware compatibility and would not repair the empty-snapshot bypass. Treating a fresh
enumeration as authority would repeat the same time-of-check assumption.
Clearing bridge BME lacks a proof that downstream forwarding stops. Leaving
translation off between candidates makes fallback depend on mutable endpoint
presence. A durable empty root gives every unselected or newly appearing source
an absent context independently of when configuration space reports it.
