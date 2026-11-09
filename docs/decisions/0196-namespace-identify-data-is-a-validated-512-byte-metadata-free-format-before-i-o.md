# D-196 Namespace Identify data is a validated 512-byte, metadata-free format before I/O queues exist

Status: Accepted, 2026-08-27. Amends D-169 and D-195; closes PW-204.

**Context.** `Controller::identify_namespace` copied `NSZE`, masked the low
four bits of `FLBAS`, selected an `LBAF` entry without checking `NLBAF`, and
evaluated `1u32 << LBADS`. A zero namespace became an apparently usable device.
An unavailable or out-of-table format was read as if supported; a hostile
`LBADS` of 32 panicked in debug and used an invalid shift in release; and the
driver silently discarded both per-LBA metadata and enabled protection
information. I/O queues were then created from those unproved fields.

The current storage caller is deliberately narrower than NVMe. D-170 accepts
only 512-byte FAT sectors and uses one fixed sector buffer. The driver supplies
no metadata pointer, does not include metadata in its transfer lengths, does
not implement protection information, and never enables the Host Behavior
Support feature's 64-format LBA extension. Accepting a format outside that
contract would not be compatibility; it would tell the controller and the
filesystem different byte extents.

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

**Rationale.** Supporting 4 KiB blocks would require a storage caller and FAT
policy which consume them; supporting metadata or protection information would
require command fields, buffers, transfer lengths and verification which do not
exist. Those are future features with concrete prerequisites, not encodings to
accept optimistically. Parsing in `libs/nvme` makes every byte-domain and
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
Nine parser defects each reddened their exact host check. Replacing a refused
device response with a synthetic valid namespace reddened only `invalid nvme
namespace stops before io queues`, with `passed=200 failed=1 expected=201`;
the interceptor observed the bypass before allocation or MMIO. The release
image then passed all 201 checks and the complete CI matrix; the mutation
register now contains 117 entries. No unsafe operation was added.
