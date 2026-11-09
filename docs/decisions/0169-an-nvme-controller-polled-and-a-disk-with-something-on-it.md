# D-169 An NVMe controller, polled, and a disk with something on it

Status: Accepted (2026-08-25) (amended by D-181)

**Context.** `012` §2 disqualified virtio-blk rather than deprioritising it:
VMware exposes no virtio device of any kind, so under D-18 it could never close
a phase, and none of the virtqueue work carries to a driver that could. AHCI
spans both hosts and is a worse first driver for the same prerequisites. NVMe
is the only overlap between the two boot targets with a freely published
specification.

**Decision.** One controller, one I/O queue pair, polled, read-only.

**Polled is what makes it affordable.** `Create I/O Completion Queue` with
`IEN = 0` disables interrupts for that queue, which removes "this kernel has no
interrupt sharing" from the critical path entirely. The cost is that a read
spins. Slice 9 gave Read a submission/completion shape but did not make device
service deferred: `Queue::read` still reaches the VFS and polls NVMe inline
while `RingEnter` serves the queue. Only Timer currently exercises in-flight
completion.

**`CAP` is read, never assumed.** The queue size, the doorbell stride, the
timeout and the page-size range all come from it, and every one changes what a
correct driver does. `012` §11 records that VMware's values are unmeasured, so
a driver that assumed QEMU's would be a driver that fails the phase's
definition of done.

**The ring parallel is a trap, and the arithmetic is shared without the
structure** (`012` §3). Four things differ from `kernel::queue` and each is a
bug if assumed away: there is no completion tail to read, so entries are found
by their **phase tag**; completions are matched by **command identifier**, not
by slot; **capacity is `size - 1`**, whose symptom is a hang; and the doorbell
is **write-only**, so the tail is shadowed. `libs/nvme` holds the arithmetic;
there is no shared `Ring<T>`.

**Rationale, and the two places a check could not have failed.**

**A disk of zeros proves nothing.** The first version of the read check read
block zero out of a zero-filled backing file and compared it to zero — which is
what a buffer nobody wrote also contains. So `xtask` writes a **different
signature into every block**, and the check reads four of them: a driver that
ignored the logical block address would answer block zero to every request and
pass against a uniform disk. D-155's rule applied to a corpus rather than to an
argument.

**Reading one block too many is invisible at the front.** Every count in this
specification is zero-based, so `CDW12` carries `blocks - 1`. A driver that
sent the count unadjusted reads *two* blocks, and the first block's bytes are
still exactly right — the check went green against that mutation. It now clears
the buffer, reads one block, and requires the bytes **after** it to still be
zero. The extent of a read is only observable past its end.

**The queue wrap needs more commands than the queue has entries.** The phase
tag flips on every wrap, and a driver that never flipped it works perfectly
until the first one and then sees either every entry or none. The check does
three times the queue's depth, because the seam is the only place the defect
exists.

**Consequences.**

- `libs/nvme`: register offsets, `CAP` decoding, the `size - 1` capacity rule,
  doorbell placement, command building, completion decoding, and PRP
  construction. Thirteen host tests and three doctests, six deliberate breaks,
  all red.
- `kernel::nvme`: the initialisation sequence, `Identify Namespace`, one I/O
  queue pair with `IEN = 0`, and a read. Four machine checks, five breaks, all
  red.
- **The register window is mapped uncacheable**, which `map_device` already
  chose, and there is now a check that says so. `012` §2 names write-back
  mapping as a silent, hard-to-debug bug; an emulator forgives it and hardware
  does not, which is exactly the class of thing that has to be caught here.
- The check machine gains an NVMe controller and a two-megabyte backing file.
  The file is the *machine's*, not the disk image's: nothing on it is booted
  and the kernel writes nothing to it.
- The signature is written by `xtask` and looked for by `selftest.rs`, which is
  two transcriptions of one table — so a host test compares them (D-158).
- **A read still spins in the serving thread.** `POLL_ATTEMPTS` bounds it,
  because a controller that does not answer is a controller to report, and on a
  machine without bus master enabled a hang is the only symptom there is.
  D-188, D-199, D-200 and D-245 later supplied teardown and deferred VFS
  ownership; only replacing polled service with a deferred or interrupt-driven
  throughput policy remains stopped behind PW-704's measured workload trigger.
