# D-387 Live VT-d entries publish once and NVMe mappings carry direction

Status: Accepted, 2026-10-11.

**Context.** A deep release audit followed the ordinary DMA revocation proof
one step earlier, to publication. `IommuTable::write_u64` converted an entry to
little-endian bytes and reused the generic volatile byte copier. VT-d read and
write permission bits live in the low byte, so a coherent hardware walk could
observe a present entry before later stores completed its physical address.
The same audit found that every transfer mapping was bidirectional and that the
controller-selection read buffer, unlike the queue and Identify buffers, was
not cleared before its first device-written command. A hostile admitted
controller could therefore race publication into an unintended frame or read
allocator residue that its command never needed.

**Decision.** The direct-span abstraction owns a bounds- and alignment-checked
volatile qword writer. `IommuTable::write_u64` uses exactly that primitive, so
one naturally aligned x86-64 store publishes address and permissions together;
generic byte writes remain available only for ordinary host-owned payloads.
VT-d segment publication takes typed device-side access. NVMe submission queues
are device-readable, completion queues and read-command payloads are
device-writable, and the existing general mapping seam remains bidirectional
only for callers whose protocol requires both directions. The startup sector
buffer is cleared before the read mapping is installed, matching every other
device-written allocation.

**Proof.** One source contract binds the qword store, the second-level
permission derivation, all three NVMe mapping directions and the clear-before-
submit order. Mutations 1,791 through 1,802 independently restore bytewise or
premature publication, widen each propagation and NVMe-call boundary, skip the
startup clear, substitute an uncleared allocation, or add a transient broad
leaf; each must redden that exact contract before restoration.
The clear runs in a non-inlined preparation stage so its frame is gone before
the controller submission frame grows. The complete host, API, self-test,
shipped and fail-closed boot gate must pass afterward.

**Alternatives.** A fence after eight byte stores cannot revoke access a live
walker already gained through the first byte. Temporarily publishing a
not-present value and then setting permissions still requires the final address
and permission transition to be atomic. Quiescing the whole controller around
every map would serialize ordinary I/O and would not excuse excess read
authority. Clearing alone limits disclosure but leaves the IOMMU policy wider
than the command; direction alone is not a reason to hand a device stale host
bytes.
