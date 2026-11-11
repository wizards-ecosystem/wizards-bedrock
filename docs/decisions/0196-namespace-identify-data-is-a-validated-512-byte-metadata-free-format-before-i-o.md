# D-196 Namespace Identify data is a validated 512-byte, metadata-free format before I/O queues exist

Status: Accepted, 2026-08-27. Amends D-169 and D-195; closes PW-204.

**Decision.** `libs/nvme::Namespace` parses the 192-byte mandatory prefix this
policy consumes: the fixed header and all sixteen `LBAF` entries available
without the extension. A value exists only when `NSZE` is non-zero, zero-based
`NLBAF` describes no more than sixteen entries, the low `FLBAS` index is inside
that table, the reserved `FLBAS` bit is clear, the selected `LBAF` has no
reserved bits, `MS` and `DPS` are zero, and `LBADS` is exactly 9. It returns the
namespace extent and the policy's 512-byte logical-block size behind read-only
accessors. No device-controlled shift is evaluated.

The two high `FLBAS` index bits are ignored, as the NVM Command Set requires
when the format extension is not enabled; the extended-metadata placement bit
is likewise inapplicable once `MS` is zero. Relative-performance bits are
accepted because they describe a supported format rather than changing its
layout. Metadata capabilities are not metadata settings and therefore do not
make an otherwise metadata-free selected format unusable.

The kernel reads exactly that prefix out of the completed Identify DMA owner
and passes it to a single configure-and-create function. That function can set
the controller's namespace fields and create I/O queues only after it owns a
`Namespace`. In self-test builds the queue-creation seam can be intercepted
before allocation, command construction or MMIO; malformed synthetic Identify
data must never reach it.

**Rationale.** `Controller::identify_namespace` had accepted a zero namespace,
read an unavailable or out-of-table format as supported, evaluated
`1u32 << LBADS` from a hostile shift, and silently dropped metadata and
protection information before creating I/O queues from those fields. The
current storage caller is deliberately narrower than NVMe. D-170 accepts
only 512-byte FAT sectors and uses one fixed sector buffer. The driver supplies
no metadata pointer, does not include metadata in its transfer lengths, does
not implement protection information, and never enables the Host Behavior
Support feature's 64-format LBA extension. Accepting a format outside that
contract would not be compatibility; it would tell the controller and the
filesystem different byte extents. Supporting 4 KiB blocks, metadata or
protection information needs a caller, command fields, buffers and verification
which do not exist: future features with concrete prerequisites, not encodings
to accept optimistically. Parsing in `libs/nvme` makes every byte-domain and
format-table bound host-testable, while the private typed value prevents the
kernel sequence from reconstructing an unvalidated size.

**Alternatives.** Clamping `LBADS`, treating unavailable formats as 512 bytes,
or falling back to LBA format zero were rejected because each silently reads a
different address space from the one the controller reported. Deferring refusal
until FAT mount was rejected because queue memory and commands would already
have been given to an unsupported namespace. Accepting every power-of-two LBA
size was rejected because the only caller still refuses every size but 512 and
because metadata is independent of the data-size exponent.

**Consequences.** Host properties exhaust `NLBAF`, `FLBAS`, `LBADS`, selected
format control and protection byte domains, every metadata size, the zero and
maximum namespace extents, and the exact 512-byte success corpus. A machine
check feeds the production configure path malformed namespace, format,
metadata, protection and size cases while intercepting queue creation; it
requires exact errors, unchanged namespace/command state and no interception.
Replacing a refused device response with a synthetic valid namespace reddens
only `invalid nvme namespace stops before io queues`, and the interceptor
observes the bypass before allocation or MMIO. No unsafe operation was added.
