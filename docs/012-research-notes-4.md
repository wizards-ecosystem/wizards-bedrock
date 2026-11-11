# 012 — Deep Research IV: Storage, and the ABI that has to precede it

> **Historical pre-Phase-6 snapshot.** Numeric measurements, dependency and
> tool versions, and words such as "current" or "today" are observations from
> the production date below. Delivered-correction notes and
> `004-decision-log.md` govern the present repository.

Produced 2026-08-25, at the close of the Phase 0 to 5 audit and before Phase 6,
when storage was expected to become the first subsystem in this kernel that
**waits**. The delivered correction is D-149/D-164: Timer proved deferral
first, while Read retained synchronous device service inside `RingEnter`.

This pass is **decision-first**: Phase 6 has to choose an on-disk format, a
block device, file rights, a path resolver and the shape of an asynchronous
read, and D-149 put the ABI slices before the filesystem, so several choices
bind code written first. Four research angles fetched primary sources and
returned verdicts, marking any claim without a source or measurement as
inference. Marks follow `010`'s:

- **✓**: verified on this machine, by building, running or measuring it.
- **◆**: a primary source was fetched directly.
- **⊕**: two angles reached it without sight of each other.

**The pass changed a decision that had already been committed** (§10).

---

## 0. The seven questions, and the short answers

| | Answer | Confidence |
|---|---|---|
| On-disk format | **FAT32, read-only, short names first** | High — and for a reason nobody expected (§1) |
| Block device | **NVMe, polled, one queue pair** | High — virtio-blk is *disqualified*, not deprioritised (§2) |
| One ring abstraction for NVMe and the syscall ABI? | **No.** Share the arithmetic, never the structure | High ⊕ (§3) |
| File and directory rights | Zircon's shape, plus one split nobody else has | High ◆ (§4) |
| Path resolution | No `/`; `..` resolved lexically **in the kernel** | High ◆ (§5) |
| Seek position | **Nowhere.** Offsets per operation | High — the evidence is one-directional ◆ (§6) |
| VFS shape | A trait over **directories**, enum dispatch | Medium — a judgement, argued (§7) |

---

## 1. The on-disk format — FAT32, and the corpus is already in the tree

`000`'s roadmap offers "FAT32 or ext2" assuming FAT is the simpler read.
**Measured, that assumption is nearly false.** GRUB2, one project and style,
both read-only over the shared `fshelp.c` ✓:

| | code lines, non-blank non-comment |
|---|---|
| `grub-core/fs/fat.c` (FAT12/16/32 incl. long names) | **673** |
| `grub-core/fs/ext2.c` minus its ext4 extent code | **~772** |

Thirteen per cent; U-Boot's read paths agree (~1.0k vs ~1.3k). **FAT32 is not
dramatically simpler to read than ext2.**

**What decides it is the corpus.** ✓ `xtask/src/image/mod.rs` already builds an
MBR disk with a FAT32 ESP in process through `fatfs` (`default-features =
false`, chrono off). Two images built seconds apart hash **identically**,
because `fatfs` defaults `volume_id` to a constant and, without chrono,
timestamps pin to the DOS epoch. Turn chrono on and the same builds differ. So
the Phase 6 test corpus is **the boot image `cargo xtask` already produces**,
reproducible at zero new tooling. On the ext2 side all three pure-Rust image
builders failed here ✓ (`ext4-mkfs` 0.1.0 needs a C compiler; `ext2` 0.1.1 has
48 compile errors and no mkfs; `tpdenk/mkfs` exits 0 and writes no file), and
the reference read-only crate `ext4-view-rs` shells out to `mkfs.ext2` and
`debugfs`, unavailable under D-16's no-system-installs rule; a frozen corpus
that cannot be regenerated cannot grow.

*The patents are dead* ◆: US5758352A, US5579517A, US6286013B1 and US5745902A
read "Expired - Lifetime" (2013 to 2015), and Microsoft's licensing page has
**no FAT or FAT32 programme**. FAT32 including long-name read and write is
unencumbered; exFAT has live patents, so do not implement exFAT. *The kernel
would read the partition it booted from* ◆: UEFI 2.10 §13.3 requires firmware
FAT32 support.

**Long names can wait.** `xtask` does **not** enable `fatfs`'s `lfn` feature,
so the ESP holds 8.3 names only and a short-names-first driver matches the
corpus; enabling `lfn` later adds coverage. Skipping long entries is one
attribute compare; GRUB's full read support is about forty-five lines.

**The counter-argument is real.** ext2 has POSIX-shaped metadata (inode
numbers, hard links, symlinks, mode bits) and FAT has none, and Phases 7 to 11
run on this driver behind the VFS WizFS later plugs into. Mitigation: design
the VFS against WizFS semantics and have the FAT driver **synthesise inode
numbers from directory-entry location**, as GRUB and Linux's `fs/fat` do. And
the corpus comes from one writer, so budget for a couple of foreign-produced
FAT32 images.

**Comparable projects** ◆: bespoke format plus host packer (xv6, `fs.c` 741 +
`mkfs.c` 307; rCore's easy-fs; RedoxFS; Theseus in-memory); ext2 (Asterinas
10,390 lines, Maestro 2,097, both chasing the Linux ABI); FAT32 via
`rust-fatfs` (ArceOS, to boot from something real). Phase 6 is the third case.

**Crates: use none of them in the kernel.** ⊕ Write the reader, and keep
`fatfs` on the *host* side as the oracle the kernel driver is checked against.
That is a better arrangement than a dependency: two independent
implementations of one format, one mature, with the disagreement between them
as the test.

---

## 2. The block device — NVMe, polled, and virtio-blk is disqualified

**virtio-blk is unusable, not a simpler first step.** ◆ Broadcom's controller
list for VMware VMs (BusLogic, LSI Logic Parallel and SAS, PVSCSI, AHCI SATA,
IDE, NVMe) has **no virtio device of any kind.** D-18 makes a VMware boot every
phase's definition of done, so a QEMU-only device could never close Phase 6,
and none of its virtqueue work would carry over. AHCI spans both hosts but is a
worse first driver (port state machine, FIS structures, command list, table and
PRDT, for the same PCI and DMA prerequisites). **NVMe is the only overlap with
a freely published, versioned specification.**

**"Polled" makes it affordable.** ◆ `Create I/O Completion Queue` with `IEN=0`
disables interrupts for that queue and the driver polls the phase tag, taking
"this kernel has no interrupt sharing" off the critical path; interrupt
coalescing resets to disabled.

**Initialisation** ◆ (NVMe Base 2.2 §3.5.1, twelve steps): wait `CSTS.RDY=0`;
set `AQA`/`ASQ`/`ACQ`; read `CAP.CSS`, set `CC.CSS`; set `CC.AMS`/`CC.MPS`;
`CC.EN=1`; wait `CSTS.RDY=1`; Identify Controller; command-set identify; Set
Features "Number of Queues"; create the I/O CQ; create the I/O SQ; async
events. Read-only drops most of the tail but not Identify Namespace (without
`NSZE` and `FLBAS → LBAF[n].LBADS` an LBA cannot be interpreted), `MDTS`
(exceeding it aborts the command), or Number of Queues, which §5.1.25.2.1
requires **before** any I/O queue is created or the create fails with Command
Sequence Error.

**What §3.5.1 does not say, and it hangs silently.** ◆ The PCI Command register
(offset `04h`) needs bit 2 (Memory Space Enable) and **bit 3 (Bus Master
Enable)**. Without BME the controller cannot DMA: queues never advance and
there is no error anywhere to read. This belongs in a comment at the write
site.

**ECAM is not required, and CF8/CFC is the safer choice.** ◆ Mechanism #1
reaches offsets `00h` to `FFh`, everything a read-only driver needs; only
extended capabilities at `100h` and above need ECAM. QEMU's `pc`/i440fx
publishes no MCFG and VMware emulates a 440BX, so CF8/CFC (about forty lines of
port I/O, no ACPI work) is the only mechanism guaranteed on **both** boot
targets. An MCFG path is purely additive later. Enumeration must **recurse
through PCI-to-PCI bridges** (VMware uses `pciBridge0` to `pciBridge7`) and
matches class `01h`, subclass `08h`, prog-IF `02h`.

**DMA on x86-64 is coherent, and that is architectural.** ◆ Intel SDM Vol 3A
§11.3.2: *"An I/O agent can perform direct memory access (DMA) to write-back
memory and the cache protocol maintains cache coherency."* Corroborated by
Linux, where x86's `__dma_wmb()` and `__dma_rmb()` are `barrier()` — compiler
barriers, no instruction emitted. **No CLFLUSH, no WBINVD, no uncached DMA
buffers.**

Still required: physical contiguity for queues (`PC=1`); page alignment per
`CC.MPS`, set to 0 to match the CPU page size; and the PRP rules of §4.3.1
(PRP1 may carry a dword-aligned offset; PRP2 is reserved, a page pointer or a
list pointer by how many page boundaries the transfer crosses; **entries
inside a PRP list must have offset zero**, else status `13h`). SDM §8.2.2 says
writes are not reordered with writes, so a release compiler fence before the
doorbell is sufficient, which is what Linux's x86 `dma_wmb()` is. **But the BAR
must be mapped uncacheable**: write-back is a real, silent, hard-to-debug bug.
Doorbells are write-only, so the tail must be shadowed in software. **No IOMMU
means no protection**: a wrong PRP is an arbitrary write into RAM, the
argument for PRP construction behind a checked `libs/` type shaped like
`frame_space::FrameSpace`.

**Crates: write it.** ◆ `nvme-driver` 0.8.1, the only maintained no_std NVMe
crate, is **IRQ-driven only** (no completion-polling helpers), wants two MSI-X
vectors, and pulls in a driver framework (`rdif-block`, `dma-api`, `mmio-api`,
`tock-registers`); depending on it would also put the `unsafe` in someone
else's crate, sidestepping D-41 rather than satisfying it. `nvme` 0.2.2 is
dead (all versions yanked). `vroom` is std + VFIO; its thesis is the useful
part. `virtio-drivers`' `Hal` trait (`dma_alloc`, `dma_dealloc`,
`mmio_phys_to_virt`, `share`, `unshare`, `BufferDirection`) is a model for the
DMA abstraction this kernel needs regardless. Estimate: 600 to 900 lines split
between a `libs/` crate for the arithmetic and `wost` for MMIO and port I/O.

---

## 3. The ring parallel — the invariant generalises, the structure does not ⊕

NVMe is a submission/completion ring with a doorbell, as is this kernel's ABI.
One abstraction for both is the obvious temptation; the verdict is **no**,
reached independently by two angles.

**The real parallel** ◆ (NVMe Base 2.2 §3.3.1.2.1): *"If there are no free slots
in a Completion Queue, then the controller shall not post status to that
Completion Queue until slots become available."* That is D-79's guarantee for
the syscall ring: never admit work whose completion has nowhere to go. One
proof obligation, stated once, is genuine shared ground.

**Where it breaks** ◆:

1. **The completion tail is not visible.** §3.3.1.2: *"The Completion Queue
   Tail pointer is only used internally by the controller and is not visible to
   the host."* An abstraction over a shared head/tail cannot express NVMe's CQ.
2. **The phase tag is not a tail index.** §4.2.4 requires the host to
   initialise every CQE's phase bit before enabling the controller or creating
   the queue; it inverts on each wrap.
3. **Completions are unordered and correlated by CID, not by slot.** The SQ
   head arrives in the `SQHD` field of arbitrary CQEs, and a new `SQHD` *"does
   not indicate either execution or completion of any command"*.
4. **The doorbell is a write-only MMIO store to uncacheable memory**; writing an
   invalid value deletes the queue.
5. **Capacity is one less.** §3.3.1.5: *"The queue is Full when the Head equals
   one more than the Tail. The number of entries in a queue when full is one
   less than the queue size."* Forcing one abstraction over a ring that allows
   full utilisation introduces a one-slot discrepancy whose symptom is a hang
   rather than an error.

**Recommendation.** Put the admission arithmetic (full given head, tail and
size; may I submit given the reserved completions) in a `libs/` crate where
`cargo test` and a D-28 mutation reach it and §3.3.1.5's `size − 1` rule is
proved once. Keep byte layout, phase tag, doorbell and PRP construction as
NVMe-specific code in `wost`, the split already made for `port_space` and
`frame_space`. **Do not build a shared `Ring<T>`**: it reads as elegance and
costs a rewrite.

---

## 4. Rights for files and directories — one split worth stealing

**The four universal rights are, bit for bit, Zircon's basic rights** ◆
(`DUPLICATE`, `TRANSFER`, `WAIT`, `INSPECT`), independent corroboration of
D-78's universal cut. **`fuchsia.io`'s nine operation bits** ◆, from
`rights-abilities.fidl`: `CONNECT`, `READ_BYTES`, `WRITE_BYTES`, `EXECUTE`,
`GET_ATTRIBUTES`, `UPDATE_ATTRIBUTES`, `ENUMERATE`, `TRAVERSE`,
`MODIFY_DIRECTORY`; *"Rights are properties specific to a connection"* versus
*"Abilities are properties intrinsic to a node."*

**Steal the `ENUMERATE`/`TRAVERSE` split**: Unix's `r` versus `x` on a
directory, "may list the names" versus "may resolve a name I already know". It
is **a real kernel distinction by this project's rule**, because the resolver
consults one and readdir the other. Capsicum has `CAP_LOOKUP` but folds
enumeration into `CAP_READ`.

**Capsicum is the counter-example D-78's rule was written about** ◆:
`CAP_FCHMOD` *"Permit fchmod(2)."*, `CAP_FPATHCONF`, `CAP_FSTATFS`; roughly two
thirds of about sixty rights are syscall names, and FreeBSD ships aliases
(`CAP_PREAD = CAP_READ|CAP_SEEK`) that concede the granularity was wrong. Three
Capsicum rights are real distinctions:

- **`CAP_LOOKUP`** (*"Permit the file descriptor to be used as a starting
  directory."*) is `TRAVERSE`: it makes a directory handle a **resolution
  root** rather than merely a readable object.
- **The source/target split** (`CAP_RENAMEAT_SOURCE`/`_TARGET`,
  `CAP_LINKAT_SOURCE`/`_TARGET`). Rename and link take *two* directory handles
  with different authority per side, and `CAP_LINKAT_SOURCE` *"allows linking
  files with broader rights"*: **folding the two into one bit is an
  authority-laundering vector.** Every survey member except Capsicum and WASI
  Preview 1 lost it.
- **`CAP_SEEK`**: moving a cursor is separable authority (§6).

**Why WASI Preview 2 deleted rights** ◆: `wasi-filesystem` issue #31
(sunfishcode, 2021-04-13) gives four reasons: *"complex to implement, complex
to use, and not very valuable in practice"*; *"It doesn't reflect anything in
POSIX"*; extra libc code to translate open flags into rights and
`ENOTCAPABLE` into errno; and `fd_fdstat_set_rights` becomes unnecessary. The
2022-06-30 CG meeting adds that rights were **inherited from CloudABI**.
**Preview 2 did not delete authority bits, it shrank them to three**:
`descriptor-flags { read, write, mutate-directory, … }`, and `mutate-directory`
is Fuchsia's `MODIFY_DIRECTORY`. Reasons two and three do not transfer: they
are artefacts of a POSIX shim between a capability API and an application, and
this kernel has no libc to satisfy. Reason one *is* D-78's rule. And the cost
WASI accepted, losing *"a readonly view of an otherwise writeable filesystem"*,
should not be accepted here: with a real handle table a read-only directory
handle is one mask check and the main reason to have per-type rights.

**Recommendation.**

- **File:** `READ`, `WRITE`. Reserve `EXECUTE` and do not define it: until the
  VFS can hand out an executable mapping of file bytes, it checks nothing and is
  a syscall name in disguise.
- **Directory:** `TRAVERSE`, `ENUMERATE`, `MODIFY`.
- **Split rename and link authority by side**, or accept a laundering vector.
- **One set, monotone non-increasing.** Drop WASI Preview 1's base/inheriting
  pair; Fuchsia's invariant (*rights are never increased*; a nested open may
  request the same or fewer) is one `u32` and one comparison.
- **`APPEND` and `RESIZE` only if enforced.** Zircon separates
  `ZX_RIGHT_RESIZE` from `ZX_RIGHT_WRITE`; `fuchsia.io` folds resize into
  `WRITE_BYTES`; neither has an append right.
  An append-only handle is a real distinction if Phase 6 has such a caller and
  a syscall name if it does not.

**Delivered correction (D-173, D-181).** OpenAt does not accept a requested
mask. Same-type directory rights are preserved; a file preserves universal
rights already held and derives `READ` from `TRAVERSE`. Current File admits
only `READ` and Directory only `TRAVERSE | ENUMERATE`. `WRITE` remains live for
Channel, while File `WRITE`, Directory `MODIFY`, and all `EXECUTE`/`MAP` uses
wait for operations that can enforce them.

---

## 5. Path resolution without a root

**`openat` fixes only the starting point.** ◆ After the first component, `..`
and symlinks escape freely, and userspace pre-sanitisation is TOCTOU-racy by
construction. Only the kernel can enforce restrictions **during** traversal.
`openat2` offers a menu ◆: `RESOLVE_BENEATH` errors on non-descendants,
`RESOLVE_IN_ROOT` clamps, `RESOLVE_NO_SYMLINKS`, `RESOLVE_NO_MAGICLINKS` (the
runc CVE, which *"exploited /proc/PID/exe links to open the runc binary for
write access"*), `RESOLVE_NO_XDEV`. **Capsicum makes `RESOLVE_BENEATH` the only
semantics** ◆, already this project's situation because there is no global
namespace to opt out of: in capability mode a path *"must not be an absolute
path and must not contain '..' components which cause the path resolution to
escape the directory hierarchy"*, and `AT_FDCWD` gives `ECAPMODE`. **Fuchsia is
the closest match** ◆ (*Dot Dot Considered Harmful*): servers **do not implement
`..` at all**, because with it *"a handle to a directory can be upgraded
arbitrarily to access the entire filesystem"*; `..` is canonicalised
client-side.

**Recommendation: three rules.**

1. **There is no `/`.** Reject absolute paths at the syscall boundary as a
   *malformed argument*, not a permission error: there is no root to resolve
   against, so this is structural rather than a check.
2. **Resolve `..` lexically, in the kernel, before touching any node**
   (Fuchsia's answer moved server-side, since this kernel has no trusted client
   library). A component vector that pops on `..` and errors on underflow means
   **the resolver never holds a parent pointer, so there is no operation that
   can walk upward**, stronger than `RESOLVE_BENEATH`, which constrains a
   resolver still capable of ascending.
3. **Walk node to node, holding a reference per step, never re-deriving a node
   from a name.** The classic TOCTOU gap exists because Linux resolves names
   against a mutable tree between check and use; with an owned node reference
   per step there is no gap to race.

An ancestor renamed mid-walk cannot grant authority under rule 2, because the
subtree moves with you and there is no way to name what is above it. The
mutation-testable invariant: **no sequence of path operations from a directory
handle can yield a node that is not a descendant of that handle's node at some
point during the call.** Defer symlinks; if added, resolve them in the same
component loop with a bounded expansion counter and reject targets that are
absolute or underflow the stack, WASI's algorithm.

---

## 6. The seek position — nowhere

**No cursor in the kernel.** Offsets per operation, no `seek`, no `tell`; for
directory enumeration only, the cursor goes to the caller as an opaque cookie.
The evidence is one-directional ◆: POSIX keeps the offset in the open file
description, not the descriptor and not the inode, so `dup` and `fork` inherit
it (`dup(2)`: *"share file offset and file status flags"*); Capsicum makes
cursor movement separate authority (`CAP_SEEK`); Fuchsia makes it
per-connection, breaking `dup` semantics anyway; **WASI Preview 2 deleted it**
(`descriptor.read(length, offset)` *"reads without updating internal state"*);
**9P never had one**.

Why it is right here: (1) a shared cursor is a lock on the hot path (Linux takes
`f_pos_lock` on every positional read of a shared descriptor); (2) the cursor is
the **only** observable coupling between two handles to one file, which in a
capability system is a covert channel and an aliasing-bug generator; (3)
per-operation offsets build a userspace cursor, while race-free `pread` cannot
be built from a shared cursor without a kernel lock; (4) it removes rights bits
that by D-78's rule would be syscall names.

**The exception.** Directory entries are variable-length and the set mutates,
so 9P and Fuchsia keep a cursor there. Return an opaque `u64` cookie from
`read_dir` and require it back: the cursor lives in the caller's memory,
`Rewind` is unnecessary, and the kernel keeps **zero** position state on any
handle; the cookie's meaning is the backend's. **Append is not a cursor**: an
offset chosen under the file's lock at write time reintroduces no readable
position.

---

## 7. The VFS shape — a trait over directories, and enum dispatch

**Do not import Linux's shape.** Its four object types, five operation tables
(`super_operations`, `inode_operations`, `dentry_operations`,
`file_operations`, `address_space_operations`) and the dcache mostly serve **a
global namespace with a mount tree** and **a shared path-lookup cache**, which
this project does not have. Importing the shape without the requirements is how
a VFS layer becomes premature. **Plan 9 and Fuchsia** ◆ make the protocol the
abstraction (*"The kernel has no knowledge about files, directories, or
filesystems"*; MemFS, MinFS and BlobFS are processes speaking one protocol).

The polymorphism is already free: a directory handle arrives over a channel and
the receiver cannot tell tmpfs from devfs from a disk, so only *where dispatch
happens* is open. **Use an enum while all three backends are in-kernel**:
monomorphic, allocation-free, exhaustively checked when a fourth arrives.
Promote to dynamic dispatch only when a filesystem lives in userspace and speaks
the channel protocol, at which point the protocol is the abstraction.

**The minimum abstraction is over directories, not files**: resolution is the
only code that must be backend-agnostic, and it needs `lookup(name)`,
`enumerate(cookie)`, and `modify` gated on `MODIFY`, matching 9P's
walk/read/remove.

**Delivered correction (D-171, D-181).** The current enum-dispatched surface
resolves, reads and enumerates. No inert `modify` method or admitted `MODIFY`
right was kept before a namespace mutation path exists. devfs lookup yields an
internal `Node::Device`; OpenAt refuses it until Phase 7 defines the
user-visible Device handle route.

**Do not force devfs through a byte interface.** A devfs entry is a name that
yields a handle of another type; making it implement `read_at`/`write_at` is
the "everything is a file" conflation WASI's README rejects: *"Unix's
'everything is a file' philosophy is in conflict with the goals of supporting
modularity and the principle of least authority."* Let devfs `lookup` return a
device handle, and keep a small byte trait for the two file backends.

---

## 8. The async file path — and how few threads it needs

**io_uring's punt rule** ◆ (`io_uring.c`): issue with `IO_URING_F_NONBLOCK`; on
`-EAGAIN`, arm a poll handler; only if that is aborted does the request go to a
worker. So work is punted when it returned `-EAGAIN`, the caller did not ask for
nonblocking, **and the file is not pollable**. Regular files are never pollable,
so a page-cache miss always ends in a worker; sockets and pipes almost never do.
`IOSQE_ASYNC` is an override for work the application *knows* will block, and
the man page warns it *"should not be used liberally"*; it is not a default. The
5.10 async-buffered-reads series added a page-unlock callback so a cache-warm
buffered read never touches a worker (12.054s → 9.283s on 4 GB in 4 K chunks).
**io_uring's history is a steady march away from its worker pool, not toward
it.** Its bounded pool defaults to `min(sq_entries, 4 × nr_cpus)`, the unbounded
one to `RLIMIT_NPROC`, both far outside a sixteen-slot budget.

**Recommendation for sixteen thread slots** (inference, marked):

1. **Always try inline with a nowait flag.**
2. **For anything pollable, register a wait, never a thread.**
3. **Only a sleeping regular-file path needs a thread.** With a
   submit-plus-completion block layer, drive the read as a **state machine** and
   the answer may be **zero workers**: the direction Linux was travelling and
   could not finish only because it had to keep every existing filesystem
   working. This kernel has one.
4. **If workers are needed, one or two dedicated slots with a bounded queue,
   and queue-full is a submission-time rejection**: a punt-capable operation
   reserves a worker slot the way it reserves completion space. No growable
   pool.
5. **Copy the submission entry at punt time**, since the ring entry may be
   overwritten before the worker runs.

**Delivered limit (D-169, D-181; amended by D-200).** Timer first implemented
the deferred state-machine shape. Read now enters one bounded worker queue;
queued and running states couple cancellation and teardown to the terminal
completion. The worker still polls an interrupt-disabled NVMe queue.

**Cancellation** ◆: `io_uring_cancelation(7)` says it *"is inherently racy"* and
**two CQEs are generated**, one for the cancel and one for the target, **order
unspecified**. That validates this project's invariant: cancel is its own
operation with its own reservation and completion, never a substitute for the
target's. `-EALREADY` means *found but too far along*, and **the caller must
still wait for the target's completion**. It is not a failure: "your completion
is coming, just not because of you." Cancel first reached only worker-punted
work, and unification took until 5.19: **build one cancel that can address any
in-flight operation from the first day.** The recurring bug class was **two
paths completing the same request** (CVE-2021-20226, CVE-2022-29582); "exactly
one terminal completion" is the antidote, structurally enforced by a consumed
reservation token, not a convention.

**Provided buffers** ◆ serve reads whose size is unknown in advance; the
ring-mapped form replaced the opcode form because *"fragmentation in the buffer
space can make it difficult to supply enough buffers at the time"*. Relevant
here only if an operation's length is unknown at submission. If ever added:
**`-ENOBUFS` is a completion-time resource failure, structurally identical to
the `-ENFILE` this project just designed away for handles.** Reserve at
submission, or write down why this class fails late.

---

## 9. The time page — two regions, not one

**Linux's vvar page** ◆ (`include/vdso/datapage.h`): a `vdso_clock` per
clocksource base, each with **its own `seq`**, holding `clock_mode`,
`cycle_last`, `mask`, `mult`, `shift` and the base timestamps,
cacheline-aligned; `nsec` is **pre-shifted** so a reader does one multiply-add,
one shift, no division. **The reader protocol** ◆: spin while `seq` is odd;
`smp_rmb()`; read every field; `smp_rmb()`; re-read `seq` and retry if it
changed. Skip the odd check and a `mult` from one epoch meets a `cycle_last`
from another; omit the **first** `smp_rmb()` and a field load hoists above the
seq load; omit the **second** and the seq re-read floats above the data reads,
the one people miss because it looks redundant; without `READ_ONCE` the
compiler merges the two seq loads.

**A writer preempted while `seq` is odd makes readers spin unboundedly.** ◆
Linux tolerates the naive spin only because its timekeeping update runs with
interrupts off. If this kernel's writer can be preempted, the protocol must
change, not the comment.

**A TSC reader also needs** ◆: a `clock_mode` gate read first, the kernel's
**revocation switch** for an unstable TSC or a resume, sending every reader to
the syscall; a masked read (`rdtsc & S64_MAX`, so backwards motion shows as a
huge delta rather than a wrap); a masked delta; an explicit backwards-motion
test returning the base timestamp; an overflow guard for long idle. **Every
failure path ends in a syscall fallback.**

**Zircon is simpler and not equivalent** ◆: its vDSO *"uses a simplified layout
that has no writable segment and requires no dynamic relocations"*, with
`vdso_constants` written once at boot, so **no seqlock, because there is no
mutable shared state**, but a constants page cannot say "the TSC was
recalibrated".

**Recommendation: both, in two regions.** An **immutable constants region**
(tick rate, page size, ABI version) with no seqlock or version counter; and a
**versioned mutable clock region**: per-clock `seq`, Linux's protocol verbatim
including *both* barriers, a `clock_mode` revocation gate, `nsec` pre-shifted,
and an always-reachable syscall fallback. D-152 chose a global page; this is
what goes in it.

**Subsequent correction (D-166, D-181).** The shipped page stores whole
`base_ns` plus a fractional carry because literal pre-shifting overflows;
revocation falls back through ring Uptime, and the C consumer's correlated
fallback remains incomplete.

---

## 10. Corrections to decisions already taken

**D-151 cited an opcode that does not exist.** ◆ The entry said io_uring grew
"direct descriptors — `IORING_OP_ACCEPT_DIRECT`" *because* completion-time
installation exhausted the descriptor table. Both halves are wrong.

There is **no `IORING_OP_ACCEPT_DIRECT`**: it is `IORING_OP_ACCEPT` with
`sqe->file_index` set, and `io_uring_prep_accept_direct()` is a liburing helper,
not an ABI entry. And the motivation was never exhaustion: Begunkov's 5.15 cover
letter, the `IORING_FILE_INDEX_ALLOC` commit message and
`io_uring_registered_files(7)` all give **per-operation overhead and
round-trips**, and none of them mentions backpressure or table limits.

**The conclusion survives on a better argument.** io_uring can install at
completion only because `IORING_FEAT_NODROP` gives it an **unbounded kernel-side
overflow backlog**. D-79 refused that for reservation admission, so
completion-time installation has nowhere to put a failure: reserving is not a
preference, it is what the earlier decision implies. io_uring's own source
agrees: `io_accept_prep()` refuses multishot with a *specific* fixed slot,
because one submission producing unbounded installs is the case that forces
kernel-managed allocation, and `IORING_FILE_INDEX_ALLOC` still allocates at
completion and reports `-ENFILE` in `cqe->res`, at the one moment the submitter
can do nothing about it. Corrected in the log and in `013` before this document
was written.

**D-153 needs one addition: exit must be a wait, not a signal.** ◆ Closing an
io_uring instance cancels pending requests, but until Axboe's March 2025 series
*"Cancel and wait for all requests on exit"*, those cancellations happened
**out of line**, so ring exit did not mean the I/O had finished; the symptom
was **mount points staying busy after the test process had exited**. For a
zombie that collects in-flight operations:

- **Exit must be a wait.** Six years of io_uring did it the other way and it
  was a bug.
- **Do not put a grace-period-based refcount on the teardown path**: the fix
  replaced a `percpu_ref` with a plain atomic, because the RCU grace period was
  too slow there.
- **The zombie must hold every resource any in-flight operation can still
  touch.** The busy-mount symptom is what happens when it does not.
- **Uncancelable operations exist.** "Cancel is a request; exit is a wait" are
  different guarantees and only the second is total.

---

## 11. What this pass could not verify

- **Whether VMware publishes an ACPI MCFG table**: no primary source, which is
  why CF8/CFC is the *safe* recommendation; treat ECAM as unavailable under
  VMware until measured.
- **VMware's actual `CAP.DSTRD`, `CAP.CQR`, `CAP.MQES`, `CAP.MPSMIN/MAX` and
  `CAP.CSS`.** Read at boot, never assumed; the circulating figures are
  secondary.
- **Whether VMware places the NVMe function on bus 0 or behind a bridge**:
  recursive enumeration makes the question go away.
- **Whether GRUB's C line counts translate to Rust.** 673 versus 772 is
  measured; the ratio carrying over is not.
- ~~**The backslash make code**, `0x5C` versus `0x5D`.~~ **Resolved by D-162:**
  `0x5D` is the set 2 value this kernel consumes; `0x5C` was the adjacent set 3
  column. Two independent tables now anchor the implemented code.
- **`lore.kernel.org` and `git.kernel.org` are behind an anti-bot wall**, so the
  io_uring findings come from a mirror, GitHub, patchwork, LWN and man7, and
  rest on **v2 patch postings rather than merged commits**.
- **A negative cannot be proven on the io_uring claim**: no source read ties
  direct descriptors to table exhaustion, but an unread list thread could.
- **Zircon's monotonic clock path**: whether `zx_clock_get_monotonic()` avoids a
  kernel entry is unconfirmed.
- **Fuchsia's exact enforcement point for `ENUMERATE`** came through a
  summarising fetch of encoded source.
- **`wiki.osdev.org` returned 403 to every request**; nothing above depends on
  it.
