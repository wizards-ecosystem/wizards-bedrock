# D-387 Live VT-d entries publish once and NVMe mappings carry direction

Status: Accepted, 2026-10-11.

**Decision.** The direct-span abstraction owns a bounds- and alignment-checked
volatile qword writer. `IommuTable::write_u64` uses exactly that primitive, so
one naturally aligned x86-64 store publishes address and permissions together;
generic byte writes remain available only for ordinary host-owned payloads.
VT-d segment publication takes typed device-side access. NVMe submission queues
are device-readable, completion queues and read-command payloads are
device-writable, and the existing general mapping seam remains bidirectional
only for callers whose protocol requires both directions. The startup sector
buffer is cleared before the read mapping is installed, matching every other
device-written allocation. The clear runs in a non-inlined preparation stage so
its frame is gone before the controller submission frame grows.

One source contract binds the qword store, the second-level permission
derivation, all three NVMe mapping directions and the clear-before-submit
order. Mutations 1,791 through 1,802 independently restore bytewise or
premature publication, widen each propagation and NVMe-call boundary, skip the
startup clear, substitute an uncleared allocation, or add a transient broad
leaf; each must redden that exact contract.

**Rationale.** VT-d read and write permission bits live in the low byte, so the
generic byte copier let a coherent hardware walk observe a present entry before
later stores completed its address, and a hostile admitted controller could
race publication into an unintended frame or read allocator residue through
bidirectional mappings and an uncleared selection buffer. A fence after eight
byte stores cannot revoke access a walker already gained; publishing not-present
first still needs an atomic final transition; quiescing the controller around
every map would serialize ordinary I/O without narrowing read authority; and
clearing alone leaves the IOMMU policy wider than the command, while direction
alone is no reason to hand a device stale host bytes.
