# D-169 An NVMe controller, polled, and a disk with something on it

Status: Accepted, 2026-08-25. Amended by D-181.

**Decision.** The first storage driver is NVMe: one controller, one I/O queue
pair, polled, read-only.

- **Polled is what makes it affordable.** `Create I/O Completion Queue` with
  `IEN = 0` disables interrupts for that queue, which takes "this kernel has no
  interrupt sharing" off the critical path. The cost is that a read spins.
  Slice 9 gave Read a submission/completion shape but did not make device
  service deferred: `Queue::read` still reaches the VFS and polls NVMe inline
  while `RingEnter` serves the queue. Only Timer currently exercises in-flight
  completion.
- **`CAP` is read, never assumed.** The queue size, the doorbell stride, the
  timeout and the page-size range all come from it, and `012` §11 records that
  VMware's values are unmeasured.
- **The ring parallel is a trap; the arithmetic is shared without the
  structure** (`012` §3). Unlike `kernel::queue`, there is no completion tail
  to read, so entries are found by their **phase tag**; completions are matched
  by **command identifier**, not by slot; **capacity is `size - 1`**, whose
  symptom is a hang; and the doorbell is **write-only**, so the tail is
  shadowed. `libs/nvme` holds the arithmetic; there is no shared `Ring<T>`.
- **The register window is mapped uncacheable**, which `map_device` chose, and
  a check says so. `012` §2 names write-back mapping as a silent bug an emulator
  forgives and hardware does not.

**Rationale.** `012` §2 disqualified virtio-blk: VMware exposes no virtio device,
so under D-18 it could never close a phase. AHCI spans both hosts and is a worse
first driver for the same prerequisites. NVMe is the only overlap between the
two boot targets with a freely published specification.

**Consequences.**

- **A disk of zeros proves nothing.** `xtask` writes a different stamp into
  every block and the check reads four of them, so a driver that ignored the
  logical block address fails; D-155's rule applied to a corpus. The stamp is
  written by `xtask` and looked for by `selftest.rs`, two transcriptions of one
  table, so a host test compares them (D-158).
- **The extent of a read is only observable past its end.** `CDW12` carries
  `blocks - 1`, and an unadjusted count reads two blocks with the first still
  right, so the check clears the buffer, reads one block, and requires the
  bytes after it to still be zero.
- **The queue wrap needs more commands than the queue has entries.** The phase
  tag flips on every wrap, so the check does three times the queue's depth.
- The check machine gains an NVMe controller and a two-megabyte backing file.
  The file is the machine's, not the disk image's: nothing on it is booted and
  the kernel writes nothing to it.
- **A read still spins in the serving thread.** `POLL_ATTEMPTS` bounds it,
  because a controller that does not answer is a controller to report, and on a
  machine without bus master enabled a hang is the only symptom there is.
  D-188, D-199, D-200 and D-245 later supplied teardown and deferred VFS
  ownership; only replacing polled service with a deferred or interrupt-driven
  throughput policy remains stopped behind PW-704's measured workload trigger.
