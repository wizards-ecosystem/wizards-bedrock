# 012 — Deep Research IV: Storage, and the ABI that has to precede it

Produced 2026-08-25, at the close of the Phase 0–5 audit and before Phase 6,
when storage was expected to become the first subsystem in this kernel that
**waits**. The delivered correction is D-149/D-164: Timer proved deferral
first, while Read retained synchronous device service inside `RingEnter`.

Where `010` was verification-first — "is what we already decided true" — this
pass is **decision-first**. Phase 6 has to choose an on-disk format, a block
device, a rights taxonomy for files, a path resolver, and the shape of an
asynchronous read. D-149 put the ABI slices before the filesystem, so several
of these choices are load-bearing for code that will be written first.

Four research angles, each instructed to fetch primary sources and return
verdicts rather than impressions, and each told that a claim without a source
or a measurement must be marked as inference. Marks below follow `010`'s:

- **✓** — verified on this machine, by building, running or measuring it.
- **◆** — a primary source was fetched directly.
- **⊕** — two angles reached it without sight of each other.

**The pass changed a decision that had already been committed.** §10 records it,
and it is the first thing to read if you only read one section.

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

**The recommendation is FAT32, and the deciding argument is not the one this
project expected.**

`000`'s roadmap offers "FAT32 or ext2" on the assumption that FAT is the
simpler read. **Measured, that assumption is nearly false.** Comparing
like with like — GRUB2, one project, one style, both read-only, both over the
shared `fshelp.c`: ✓

| | code lines, non-blank non-comment |
|---|---|
| `grub-core/fs/fat.c` (FAT12/16/32 incl. long names) | **673** |
| `grub-core/fs/ext2.c` minus its ext4 extent code | **~772** |

Thirteen per cent. U-Boot's read paths give the same direction and a wider gap
(~1.0k vs ~1.3k). **Anyone who says FAT32 is dramatically simpler to read than
ext2 is wrong**, and this document says so because the roadmap's phrasing
invites the belief.

**What actually decides it is the corpus.** ✓

`xtask/src/image/mod.rs` already builds an MBR disk with a FAT32 ESP, in
process, through the `fatfs` crate — `default-features = false`, chrono
deliberately off. Two images built two and a half seconds apart hash
**identically**, and the determinism is not luck: `fatfs` defaults `volume_id`
to a constant and, with chrono off, timestamps pin to the DOS epoch. Turn
chrono on and the same builds differ.

So the Phase 6 test corpus is **the boot image `cargo xtask` already
produces**, at zero new tooling and with reproducibility already established.

The ext2 side of that is not merely worse, it is absent. All three pure-Rust
ext2 image builders were tried on this machine and all three failed: ✓

- `ext4-mkfs` 0.1.0 — wraps lwext4 in C; fails to build, no compiler here.
- `ext2` 0.1.1 — 48 compile errors, libc/Unix-only, and exposes no mkfs anyway.
- `tpdenk/mkfs` — advertises this exact use; its ext2 exits 0 and writes no
  file.

The reference read-only ext2 crate, `ext4-view-rs`, generates its corpus by
shelling out to `mkfs.ext2` and `debugfs`. Under D-16's no-system-installs rule
that is not available here, and a frozen committed corpus that cannot be
regenerated is a corpus you cannot grow.

**Two further points in FAT32's favour, one of them specific to this project.**

*The patents are dead, verifiably.* ◆ US5758352A, US5579517A, US6286013B1 and
US5745902A all read "Expired - Lifetime" with anticipated expirations between
2013 and 2015. Microsoft's current technology-licensing page lists exFAT, EAS,
SMB, RDP, PlayReady and others — **there is no FAT or FAT32 programme**. FAT32
including long-name read and write is unencumbered. exFAT is the one variant
with live patents, so do not implement exFAT.

*The kernel would be reading the partition it booted from.* ◆ UEFI 2.10 §13.3
requires firmware to support FAT32. The corpus is the artefact, and the block
path gets exercised against a volume the firmware also validated.

**Long names can wait, and the existing corpus says so.** The specification's
skip test is one attribute compare; full read support in GRUB is about
forty-five lines. `xtask` does **not** enable `fatfs`'s `lfn` feature, so
today's ESP contains 8.3 names only — a short-names-first driver matches the
existing corpus exactly, and enabling `lfn` is the one-word change that adds
coverage later.

**The honest counter-argument, recorded because it is real.** ext2 has
POSIX-shaped metadata — inode numbers, hard links, symlinks, mode bits — and
FAT has none of it. Phases 7 to 11 run on this driver behind the VFS that WizFS
later plugs into, so a FAT-shaped VFS is an architectural risk. The mitigation
is standard: design the VFS against WizFS semantics and have the FAT driver
**synthesise inode numbers from directory-entry location**, which is what both
GRUB and Linux's `fs/fat` do.

A second weakness: the corpus would come from exactly one writer. Budget for
obtaining a couple of foreign-produced FAT32 images so the driver is not
validated solely against the tool that made its input.

**What comparable projects chose**, which is a cleaner pattern than expected: ◆

- Bespoke format plus a host packer — xv6 (`fs.c` 741 lines + `mkfs.c` 307),
  rCore's easy-fs, Redox's RedoxFS, Theseus (in-memory only).
- ext2 — Asterinas (10,390 lines, read/write) and Maestro (2,097), both
  chasing the Linux ABI, where POSIX metadata is a hard requirement.
- FAT32 via `rust-fatfs` — ArceOS, which wanted to boot from something real.

Phase 6 is the third case.

**Crates: use none of them in the kernel.** ⊕ Both this angle and §2's reached
the same verdict independently. Write the reader — it is the point of the phase
— and keep `fatfs` on the *host* side, where it already is, as the oracle the
kernel driver is checked against. That is a better arrangement than a
dependency: two independent implementations of one format, one of which is
mature, with the disagreement between them as the test.

---

## 2. The block device — NVMe, polled, and virtio-blk is disqualified

**virtio-blk is not a simpler first step. It is unusable.** ◆

Broadcom's own controller list for VMware VMs is BusLogic Parallel, LSI Logic
Parallel, LSI Logic SAS, VMware Paravirtual SCSI, AHCI SATA, IDE, and NVMe.
**There is no virtio device of any kind.** D-18 makes a VMware boot every
phase's definition of done, so a QEMU-only device could never close Phase 6 —
and none of the virtqueue work would carry over to the driver that could.

AHCI also spans both hosts and is a worse first driver: a port state machine,
FIS structures, and a command list plus command table plus PRDT, for the same
PCI and DMA prerequisites. **NVMe is the only overlap with a freely published,
versioned specification.**

**The scope that makes this affordable is "polled".** ◆ `Create I/O Completion
Queue` with `IEN=0` disables interrupts for that queue; the driver polls the
phase tag. That removes "this kernel has no interrupt sharing" from the
critical path entirely. Interrupt coalescing needs no attention either — both
its fields reset to zero, which is disabled.

**The initialisation sequence is specified, and it is twelve steps.** ◆ NVMe
Base 2.2 §3.5.1 gives it in order: wait `CSTS.RDY=0`; set `AQA`/`ASQ`/`ACQ`;
read `CAP.CSS` and set `CC.CSS`; set `CC.AMS`/`CC.MPS`; `CC.EN=1`; wait
`CSTS.RDY=1`; Identify Controller; command-set identify; Set Features "Number
of Queues"; create the I/O completion queue; create the I/O submission queue;
async events.

For read-only, most of the tail drops. What does **not** drop: Identify
Namespace, because without `NSZE` and `FLBAS → LBAF[n].LBADS` an LBA cannot be
interpreted; `MDTS`, because exceeding it aborts the command; and Set Features
"Number of Queues", which §5.1.25.2.1 requires **before** any I/O queue is
created or the create fails with Command Sequence Error.

**The one thing §3.5.1 does not tell you, and it hangs silently.** ◆ The PCI
Command register at config offset `04h` needs bit 2 (Memory Space Enable) and
**bit 3 (Bus Master Enable)**. Without BME the controller cannot DMA at all:
queues never advance and there is no error anywhere to read. This belongs in a
comment at the write site, not in a document.

**ECAM is not required, and CF8/CFC is the safer choice.** ◆ Legacy
Configuration Mechanism #1 reaches config offsets `00h`–`FFh`, which holds
everything a read-only driver needs — Command register, class code, BAR0/BAR1,
and the standard capability list. Only *extended* capabilities at `100h` and
above need ECAM, and a polled driver reads none.

The positive argument is stronger than sufficiency: QEMU's `pc`/i440fx machine
publishes no MCFG, and VMware Workstation emulates a 440BX chipset. CF8/CFC is
about forty lines of port I/O, needs no ACPI work, and is the only mechanism
guaranteed present on **both** boot targets. An MCFG path is purely additive
later.

Two enumeration details that decide whether the device is found at all:
enumeration must **recurse through PCI-to-PCI bridges**, because VMware places
functions behind `pciBridge0`–`pciBridge7`; and the match is on class `01h`,
subclass `08h`, prog-IF `02h`.

**DMA on x86-64 is coherent, and that is architectural.** ◆ Intel SDM Vol 3A
§11.3.2: *"An I/O agent can perform direct memory access (DMA) to write-back
memory and the cache protocol maintains cache coherency."* Corroborated by
Linux, where x86's `__dma_wmb()` and `__dma_rmb()` are `barrier()` — compiler
barriers, no instruction emitted. **No CLFLUSH, no WBINVD, no uncached DMA
buffers.**

What must still be guaranteed: physical contiguity for queues (`PC=1`);
page alignment per `CC.MPS`, which should be set to 0 so it coincides with the
CPU page size; and the PRP rules of §4.3.1 — PRP1 may carry a dword-aligned
offset, PRP2 is reserved / a page pointer / a list pointer depending on how
many page boundaries the transfer crosses, and **entries inside a PRP list must
have offset zero**. Violations return status `13h`, PRP Offset Invalid.

Store ordering needs no fence: SDM §8.2.2 says writes are not reordered with
other writes, so the ordinary stores building an SQE are visible before the
doorbell store. A release compiler fence is correct and sufficient — which is
exactly what Linux's x86 `dma_wmb()` is. **But the BAR must be mapped
uncacheable**; mapping it write-back is a real, silent, hard-to-debug bug, and
doorbells are write-only so the tail must be shadowed in software.

**No IOMMU means no protection**: the controller consumes host-physical
addresses directly, so a wrong PRP is an arbitrary write into RAM. That is the
argument for putting PRP construction behind a checked `libs/` type, the same
shape as `frame_space::FrameSpace`.

**Crates: write it.** ◆ `nvme-driver` 0.8.1 is the only maintained no_std NVMe
crate and is genuinely good work, but it is **IRQ-driven only** — its README
says there are no completion-polling helpers — it wants two MSI-X vectors, and
it pulls in `rdif-block`, `dma-api`, `mmio-api` and `tock-registers`: a driver
framework, not a driver. Depending on it also puts the `unsafe` in someone
else's crate, which sidesteps D-41 rather than satisfying it. `nvme` 0.2.2 is
dead — all seven versions yanked, repository 404. `vroom` is std + VFIO +
hugepages; its accompanying thesis is the valuable artefact, not its code.

`virtio-drivers`' `Hal` trait — `dma_alloc`, `dma_dealloc`,
`mmio_phys_to_virt`, `share`, `unshare`, plus a `BufferDirection` — is worth
reading as a model for the DMA abstraction this kernel has to build regardless.

Estimated 600–900 lines split between a `libs/` crate for the arithmetic and
`wost` for the MMIO and port I/O. Smaller than integrating any of the above.

---

## 3. The ring parallel — the invariant generalises, the structure does not ⊕

NVMe is itself a submission/completion ring with a doorbell, and this kernel
already has one. The temptation to build one abstraction is obvious and the
verdict is **no** — reached independently by two angles.

**Where the parallel is real, and worth writing down once.** ◆ NVMe Base 2.2
§3.3.1.2.1: *"If there are no free slots in a Completion Queue, then the
controller shall not post status to that Completion Queue until slots become
available."* That is the same guarantee D-79 chose for the syscall ring: never
admit work whose completion has nowhere to go. **The host is expected to hold
the same obligation the kernel already holds.** One proof obligation, stated
once, is genuine shared ground.

**Where it breaks, concretely.** ◆

1. **The completion tail is not yours and is not visible.** §3.3.1.2: *"The
   Completion Queue Tail pointer is only used internally by the controller and
   is not visible to the host."* An abstraction parameterised on a shared
   head/tail cannot express NVMe's CQ at all.
2. **The phase tag is not a substitute for a tail index.** It changes the
   polling discipline and carries a lifecycle requirement with no software
   analogue — §4.2.4 requires the host to initialise every CQE's phase bit
   before enabling the controller or creating the queue, after which it inverts
   on each wrap.
3. **Completions are unordered and correlated by CID, not by slot.** The SQ
   head advance arrives in the `SQHD` field of arbitrary CQEs, and a new
   `SQHD` *"does not indicate either execution or completion of any command"*.
4. **The doorbell is a write-only MMIO store to uncacheable memory.** Different
   ordering rules, different failure mode — writing an invalid value deletes
   the queue.
5. **A capacity off-by-one that would bite where debugging is hardest.**
   §3.3.1.5: *"The queue is Full when the Head equals one more than the Tail.
   The number of entries in a queue when full is one less than the queue
   size."* If the existing ring's arithmetic allows full utilisation, it is
   **not** NVMe's, and forcing one abstraction over both introduces a one-slot
   discrepancy whose symptom is a hang rather than an error.

**Recommendation.** Put the admission arithmetic in a `libs/` crate — is this
queue full given head, tail and size; may I submit given the reserved
completions — where `cargo test` and a D-28 mutation can reach it, and where
§3.3.1.5's `size − 1` rule is stated once and proved. Keep the byte layout,
phase tag, doorbell and PRP construction as NVMe-specific code in `wost`. That
is the split this project already made for `port_space` and `frame_space`.

**Do not build a shared `Ring<T>`.** It reads as elegance and costs a rewrite.

---

## 4. Rights for files and directories — one split worth stealing

**The four universal rights are, bit for bit, Zircon's basic rights.** ◆
`DUPLICATE`, `TRANSFER`, `WAIT`, `INSPECT`. That is independent corroboration
that the universal cut in D-78 is the right one, arrived at by a different
project for the same reasons.

**`fuchsia.io`'s nine operation bits, verbatim from `rights-abilities.fidl`:** ◆
`CONNECT`, `READ_BYTES`, `WRITE_BYTES`, `EXECUTE`, `GET_ATTRIBUTES`,
`UPDATE_ATTRIBUTES`, `ENUMERATE`, `TRAVERSE`, `MODIFY_DIRECTORY`. The same file
draws the distinction this design needs: *"Rights are properties specific to a
connection"* versus *"Abilities are properties intrinsic to a node."*

**The single most valuable thing to steal is the `ENUMERATE`/`TRAVERSE`
split.** It is Unix's `r` versus `x` on a directory — "may list the names"
versus "may resolve a name I already know" — and it is **a real kernel
distinction by this project's own rule**, because the resolver consults one and
readdir consults the other. Nobody else in the survey has it cleanly: Capsicum
has `CAP_LOOKUP` but folds enumeration into `CAP_READ`; WASI Preview 1 had the
two as separate bits by accident of being syscall-named.

**Capsicum is the counter-example D-78's rule was written about.** ◆ From
`rights(4)`: `CAP_FCHMOD` — *"Permit fchmod(2)."* `CAP_FPATHCONF` — *"Permit
fpathconf(2)."* `CAP_FSTATFS` — *"Permit fstatfs(2)."* Roughly two thirds of
about sixty rights are syscall names. FreeBSD itself concedes the granularity
was wrong by shipping aliases — `CAP_PREAD = CAP_READ|CAP_SEEK`,
`CAP_FCHMODAT = CAP_FCHMOD|CAP_LOOKUP`.

Three Capsicum rights *are* real distinctions and one of them matters a great
deal:

- **`CAP_LOOKUP`** — *"Permit the file descriptor to be used as a starting
  directory."* This is `TRAVERSE`, and it is what makes a directory handle a
  **resolution root** rather than merely a readable object.
- **The source/target split** — `CAP_RENAMEAT_SOURCE`/`_TARGET`,
  `CAP_LINKAT_SOURCE`/`_TARGET`. Rename and link are the only operations taking
  *two* directory handles, and the authority required differs per side. The man
  page notes `CAP_LINKAT_SOURCE` *"allows linking files with broader rights"* —
  **this is an authority-laundering vector if the two are folded into one bit.**
  Every survey member except Capsicum and WASI Preview 1 lost this distinction.
- **`CAP_SEEK`** — its existence is evidence that moving a cursor is separable
  authority, which bears on §6.

**Why WASI Preview 2 deleted rights, from the primary record.** ◆
`wasi-filesystem` issue #31, opened by sunfishcode 2021-04-13, gives four
reasons: the system is *"complex to implement, complex to use, and not very
valuable in practice"*; *"It doesn't reflect anything in POSIX"*; it *"requires
extra code in libc, both in `path_open` to translate POSIX open flags into
rights flags, and to translate from `ENOTCAPABLE` into POSIX errno values"*;
and `fd_fdstat_set_rights` becomes unnecessary. The WASI CG meeting of
2022-06-30 adds that rights were **inherited from CloudABI** and lacked the
flexibility developers needed.

**The nuance every summary misses: Preview 2 did not delete authority bits, it
shrank them to three.** `types.wit` still carries `descriptor-flags { read,
write, mutate-directory, … }` — which is exactly the residue after the syscall
names are burned off, and `mutate-directory` is Fuchsia's `MODIFY_DIRECTORY`.

**How much of that failure transfers here: reasons two and three, not at all.**
Both are artefacts of a POSIX shim sitting between a capability API and an
application, obliged to synthesise rights masks from `O_*` flags and demote
`ENOTCAPABLE` into `errno`. This kernel has no libc to satisfy and no POSIX
compatibility goal. Reason one *is* D-78's rule, arrived at independently after
two years of implementation experience.

**And the cost WASI knowingly accepted should not be accepted here.** Issue #31
explicitly gives up *"a readonly view of an otherwise writeable filesystem"*.
In a runtime with no handle table that is cheap; in a kernel with a real one, a
read-only directory handle is a mask check on one code path and is the main
reason to have per-type rights at all.

**Recommendation.**

- **File:** `READ`, `WRITE`. Reserve `EXECUTE` and do not define it — until the
  VFS can hand out an executable mapping of file bytes, an `EXECUTE` bit checks
  nothing and is a syscall name in disguise.
- **Directory:** `TRAVERSE`, `ENUMERATE`, `MODIFY`.
- **Split rename and link authority by side**, or accept a laundering vector.
- **One set, monotone non-increasing.** Drop WASI Preview 1's base/inheriting
  pair, which forced libc to compute two masks per open. Fuchsia's invariant —
  *rights are never increased*, a nested open may request the same or fewer —
  is one `u32` and one comparison.
- **`APPEND` and `RESIZE` only if enforced.** Zircon separates
  `ZX_RIGHT_RESIZE` from `ZX_RIGHT_WRITE`; `fuchsia.io` folds resize into
  `WRITE_BYTES`; neither has an append right. An append-only handle is a real
  distinction if Phase 6 has such a caller and a syscall name if it does not.

**Delivered correction (D-173, D-181).** OpenAt does not accept a requested
mask. Same-type directory rights are preserved; a file preserves universal
rights already held and derives `READ` from `TRAVERSE`. Current File admits
only `READ` and Directory only `TRAVERSE | ENUMERATE`. `WRITE` remains live for
Channel, while File `WRITE`, Directory `MODIFY`, and all `EXECUTE`/`MAP` uses
wait for operations that can enforce them.

---

## 5. Path resolution without a root

**`openat` fixes only the starting point.** ◆ Resolution after the first
component is unrestricted, so `..` and symlinks escape freely, and userspace
pre-sanitisation is TOCTOU-racy by construction — a component validated as a
directory can become a symlink before it is traversed. LWN's introduction of
`openat2` puts it plainly: *"looking up a file given a path name seems like a
straightforward task, but it turns out to be one of the more complex things the
kernel does."* Only the kernel can enforce restrictions **during** traversal.

**What `openat2` guarantees**, and it is a menu rather than a design: ◆
`RESOLVE_BENEATH` errors on anything not a descendant; `RESOLVE_IN_ROOT` clamps
instead, chroot-like; `RESOLVE_NO_SYMLINKS`; `RESOLVE_NO_MAGICLINKS`, which is
the runc CVE — the breakout *"exploited /proc/PID/exe links to open the runc
binary for write access"*; `RESOLVE_NO_XDEV`.

**Capsicum makes `RESOLVE_BENEATH` the only semantics rather than a flag**, ◆
which is already this project's situation because there is no global namespace
to opt out of. In capability mode `openat` requires that the path *"must not be
an absolute path and must not contain '..' components which cause the path
resolution to escape the directory hierarchy"*, and `AT_FDCWD` gives `ECAPMODE`.

**Fuchsia goes further and is the closest match.** ◆ *Dot Dot Considered
Harmful*: filesystem servers **do not implement `..` at all**, because *"if a
handle is provided to a directory, the client can simply request '..', and the
handle will be 'upgraded' to access the parent directory, with broader
scope… a handle to a directory can be upgraded arbitrarily to access the entire
filesystem."* `..` is canonicalised client-side before any path reaches a
server.

**Recommendation — three rules, all cheap in this architecture.**

1. **There is no `/`.** Reject absolute paths at the syscall boundary as a
   *malformed argument*, not a permission error. There is no root to resolve
   against, so this is structural rather than a check.
2. **Resolve `..` lexically, in the kernel, before touching any node.**
   Fuchsia's answer, moved server-side because this kernel has no trusted
   client library. A component vector that pops on `..` and errors on underflow
   means **the resolver never holds a parent pointer, so there is no operation
   that can walk upward.** That is stronger than `RESOLVE_BENEATH`, which
   enforces a predicate on a resolver still capable of ascending.
3. **Walk node to node, holding a reference per step, never re-deriving a node
   from a name.** The classic TOCTOU gap exists because Linux resolves names
   against a mutable tree between check and use. If each step yields an owned
   node reference, there is no gap to race.

The residual case — an ancestor renamed mid-walk — cannot grant authority under
rule 2, because the subtree moves with you and there is no way to name what is
above it. That gives a stateable, mutation-testable invariant: **no sequence of
path operations from a directory handle can yield a node that is not a
descendant of that handle's node at some point during the call.**

Defer symlinks. If Phase 6 adds them, resolve inside the same component loop
with a bounded expansion counter, and reject targets that are absolute or that
underflow the component stack — WASI's algorithm verbatim.

---

## 6. The seek position — nowhere

**Recommendation: no cursor in the kernel.** Offsets per operation, no `seek`,
no `tell` — with one narrow exception for directory enumeration, and even there
the cursor goes to the caller as an opaque cookie.

The evidence is unusually one-directional: ◆

- **POSIX/Linux** put the offset in the *open file description*, not the
  descriptor and not the inode — a third object that exists mainly so `dup` and
  `fork` inherit a cursor. `dup(2)`: the two descriptors *"share file offset
  and file status flags"*.
- **Capsicum** treats moving the cursor as separate authority (`CAP_SEEK`), and
  requires it for positional I/O.
- **Fuchsia** keeps a cursor but makes it **per-connection**, so two
  connections to one file do not share it — which breaks POSIX `dup` semantics
  anyway.
- **WASI Preview 2 deleted it.** `descriptor.read(length, offset)` *"reads
  without updating internal state"*. Sequential access is a *stream* object the
  caller holds, so the cursor lives in the caller.
- **9P has never had one.** The offset is in every `Tread` message.

**Four reasons this is right here, not merely popular.**

1. A shared mutable cursor is a lock on the hot path — Linux takes `f_pos_lock`
   on every positional read of a shared descriptor. That is a lock bought to
   implement a convenience.
2. The cursor is the **only** reason two handles to one file have observable
   coupling. In a capability system, handle A's behaviour changing because of
   an operation on handle B is a covert channel and an aliasing-bug generator.
3. Per-operation offsets are what a userspace cursor is built *from*. The
   reverse — synthesising race-free `pread` from a shared cursor — is not
   possible without a lock the kernel must provide.
4. It removes an entire class of right from the rights word, and by D-78's rule
   those bits would otherwise be syscall names.

**The exception, stated honestly.** Directory enumeration cannot be made
stateless the way byte reads can: entries are variable-length and the set
mutates under you. Both 9P and Fuchsia keep a cursor here. Return an opaque
`u64` cookie from `read_dir` and require it back — the cursor lives in the
caller's memory, `Rewind` becomes unnecessary, and the kernel keeps **zero**
position state on any handle. The cookie's meaning is the backend's business.

**Append is not a cursor.** If atomic append is wanted it is a mode where the
offset is chosen under the file's lock at write time, which does not
reintroduce a readable position.

---

## 7. The VFS shape — a trait over directories, and enum dispatch

**Do not import Linux's shape.** Four object types and five operation tables —
`super_operations`, `inode_operations`, `dentry_operations`, `file_operations`,
`address_space_operations` — plus the dcache. The overwhelming majority of that
machinery serves two things this project deliberately does not have: **a global
namespace with a mount tree**, and **a shared path-lookup cache** that must
stay coherent with it. Importing the shape without the requirements is how a
VFS layer becomes premature.

**Plan 9 and Fuchsia show the alternative.** ◆ Plan 9's abstraction *is* the
protocol — every backend is a 9P server. Fuchsia is the same over channels:
*"The kernel has no knowledge about files, directories, or filesystems"*, and
MemFS, MinFS and BlobFS are three processes speaking one protocol with no
in-kernel vtable anywhere.

**The honest judgement at three backends.**

The polymorphism is already mandatory and already free: a directory handle
arrives over a channel and the receiver cannot tell tmpfs from devfs from a
disk. So the only question is *where dispatch happens*, which is a maintenance
question, not a design one.

**Use an enum while all three backends are in-kernel.** It is monomorphic,
allocation-free, exhaustively checked when a fourth arrives, and directly
testable. Promote to dynamic dispatch only when a backend must be added without
editing the enum — which in this architecture means *when a filesystem lives in
userspace and speaks the channel protocol*, and at that point the abstraction is
the protocol, Plan 9 style, and the in-kernel trait is irrelevant.

**The minimum useful abstraction is over directories, not files.** Path
resolution is the only code that must be backend-agnostic, and it needs exactly
three operations: `lookup(name)`, `enumerate(cookie)`, and `modify` gated on
`MODIFY`. That matches 9P's walk/read/remove.

**Delivered correction (D-171, D-181).** The current enum-dispatched surface
resolves, reads and enumerates. No inert `modify` method or admitted `MODIFY`
right was kept before a namespace mutation path exists. devfs lookup yields an
internal `Node::Device`; OpenAt refuses it until Phase 7 defines the
user-visible Device handle route.

**Do not force devfs through a byte interface.** A devfs entry has no bytes; it
is a name that yields a handle of some *other* type. Making it implement
`read_at`/`write_at` to satisfy a trait is the "everything is a file"
conflation that WASI's own README rejects on exactly these grounds: *"Unix's
'everything is a file' philosophy is in conflict with the goals of supporting
modularity and the principle of least authority."* Let `lookup` on devfs return
a device handle. Keep a small byte trait for file backends, where there are
genuinely two implementations.

---

## 8. The async file path — and how few threads it needs

**io_uring's actual punt rule**, from `io_uring.c`: ◆ issue is attempted with
`IO_URING_F_NONBLOCK`; on `-EAGAIN`, arm a poll handler; only if that is
*aborted* does the request go to a worker thread. So work is punted when it
returned `-EAGAIN`, the caller did not ask for nonblocking, **and the file is
not pollable**. Regular files are never pollable, so a page-cache miss always
ends in a worker; sockets and pipes almost never do.

`IOSQE_ASYNC` is an override for work the application *knows* will block, and
the man page warns it *"should not be used liberally"*. It is not a default.

**How hard they worked to avoid workers** is the useful signal. ◆ The 5.10
async-buffered-reads series added a page-unlock callback so a cache-warm
buffered read never touches a worker; the measured result was 12.054s → 9.283s
on 4 GB in 4 K chunks. **io_uring's history is a steady march away from its
worker pool, not toward it.**

For calibration, its bounded pool defaults to `min(sq_entries, 4 × nr_cpus)`
and its unbounded pool is limited only by `RLIMIT_NPROC` — both far outside a
sixteen-slot budget.

**Recommendation for sixteen thread slots** (inference, marked):

1. **Always try inline with a nowait flag.** Free, and the common case.
2. **For anything pollable, register a wait — never a thread.**
3. **The only case genuinely needing a thread is a regular-file path that
   sleeps.** If the block layer is submit-plus-completion, drive the read as a
   **state machine** and the answer may be **zero workers**. That is the
   direction Linux was travelling and could not finish only because it had to
   keep every existing filesystem working. This kernel has one.
4. **If workers are needed, one or two dedicated slots with a bounded queue,
   and queue-full is a submission-time rejection.** That falls straight out of
   reserve-at-submission: a punt-capable operation reserves a worker slot the
   way it reserves completion space. Do not build a growable pool.
5. **Copy the submission entry at punt time.** The ring entry may be
   overwritten before the worker runs. This bit them and is easy to miss.

**Delivered limit (D-169, D-181).** Timer implements the deferred state-machine
shape. Read is a ring operation but still polls NVMe inline while `RingEnter`
serves it; the block-device state machine, interrupt path and teardown coupling
recommended above remain future work.

**Cancellation, and what it means for "exactly one terminal completion".** ◆
`io_uring_cancelation(7)` is explicit that cancellation *"is inherently racy"*
and that **two CQEs are generated** — one for the cancel, one for the target —
with **order unspecified**. That is compatible with this project's invariant
and validates it: cancel is itself an operation with its own reservation and
its own completion, and it never substitutes for the target's.

`-EALREADY` is the one to design for: it means *found but too far along*, and
**the caller must still wait for the target's completion**. It is not a failure;
it is "your completion is coming, just not because of you."

What they got wrong first is instructive: cancel was originally reachable only
for worker-punted work, so poll and timeout needed separate opcodes, and
unification took until 5.19. **Build one cancel that can address any in-flight
operation, from the first day.** And the recurring bug class was **two paths
completing the same request** — linked-timeout double completion,
CVE-2021-20226, CVE-2022-29582. "Exactly one terminal completion" is the direct
antidote; make it structurally enforced by a consumed reservation token, not a
convention.

**Provided buffers**, briefly: ◆ they solve reads whose size is not known in
advance, and the ring-mapped form replaced the opcode form because *"fragmentation
in the buffer space can make it difficult to supply enough buffers at the
time"*. Relevant here only if an operation's length is unknown at submission.
One warning if it is ever added: **`-ENOBUFS` is a completion-time resource
failure, structurally identical to the `-ENFILE` this project just designed
away for handles.** Be consistent — reserve at submission, or write down why
this one class fails late.

---

## 9. The time page — two regions, not one

**Linux's vvar page**, current `include/vdso/datapage.h`: ◆ a `vdso_clock` per
clocksource base, each with **its own `seq`**, holding `clock_mode`,
`cycle_last`, `mask`, `mult`, `shift` and the base timestamps. The whole
structure is cacheline-aligned and the hot clocks sit in the first lines.
`nsec` is stored **pre-shifted**, so a reader does one multiply-add and one
shift and no division.

**The reader protocol, exactly:** ◆ spin while `seq` is odd; `smp_rmb()`; read
every field; `smp_rmb()`; re-read `seq` and retry if it changed.

The failure modes are worth stating because two of them look like redundancy:

- Skip the odd check → read mid-update → a `mult` from one epoch with a
  `cycle_last` from another. Time jumps or goes backwards.
- Omit the **first** `smp_rmb()` → a field load hoists above the seq load.
- Omit the **second** → the seq re-read floats above the data reads. *This is
  the one people miss, because it looks redundant. It is not.*
- No `READ_ONCE` → the compiler merges the two seq loads and the retry becomes
  a tautology.

**And one that decides whether this protocol is usable here at all:** ◆
**a writer preempted while `seq` is odd makes readers spin unboundedly.** Linux
tolerates the naive spin only because its timekeeping update runs with
interrupts off. If this kernel's writer can be preempted, the protocol must
change, not the comment.

**What a TSC reader needs beyond the seqlock:** ◆ a `clock_mode` gate read
first — this is the kernel's **revocation switch** for an unstable TSC or a
resume from suspend, and every reader falls back to the syscall; a masked read
(`rdtsc & S64_MAX`, so backwards motion shows as a huge delta rather than a
wrap); a masked delta; an explicit backwards-motion test that returns the base
timestamp unmodified; and an overflow guard for the long-idle case.
**Every failure path ends in a syscall fallback.**

**Zircon is simpler by construction and not equivalent.** ◆ Its vDSO *"uses a
simplified layout that has no writable segment and requires no dynamic
relocations"*, and its `vdso_constants` are written once by the kernel at boot.
No writable segment means **no seqlock, because there is no mutable shared
state** — but that buys simplicity by giving up the thing Linux's page exists
for: publishing a *changing* transform. A constants page cannot express "the
TSC was recalibrated".

**Recommendation: take both, in two regions.**

- An **immutable constants region** — tick rate, page size, ABI version — with
  no seqlock and no version counter at all. Zircon's model, and it removes the
  most error-prone reads from the protocol entirely.
- A **versioned mutable clock region** — per-clock `seq`, Linux's protocol
  verbatim including *both* barriers, a `clock_mode` revocation gate, `nsec`
  pre-shifted, and a syscall fallback that is always reachable.

D-152 chose a global page; this is what goes in it.

**Subsequent correction (D-166, D-181).** The shipped page stores whole
`base_ns` plus a fractional carry because literal pre-shifting overflows;
revocation falls back through ring Uptime, and the C consumer's correlated
fallback remains incomplete.

---

## 10. Corrections to decisions already taken

**D-151 cited an opcode that does not exist.** ◆ The entry said io_uring grew
"direct descriptors — `IORING_OP_ACCEPT_DIRECT`" *because* completion-time
installation exhausted the descriptor table. Both halves are wrong.

There is **no `IORING_OP_ACCEPT_DIRECT`**. It is `IORING_OP_ACCEPT` with
`sqe->file_index` set; `io_uring_prep_accept_direct()` is a liburing helper,
not an ABI entry. And the merge argument was never exhaustion: Begunkov's 5.15
cover letter, the `IORING_FILE_INDEX_ALLOC` commit message and
`io_uring_registered_files(7)` all give **per-operation overhead and
round-trips**, and none of them mentions backpressure or table limits.

**The conclusion survives on a better argument**, and one this project had
already made. io_uring can install at completion time only because
`IORING_FEAT_NODROP` gives it an **unbounded kernel-side overflow backlog** for
completions that will not fit. D-79 refused exactly that in favour of
reservation admission. Having refused the backlog, completion-time installation
has nowhere to put a failure — so reserving is not a preference, it is what the
earlier decision already implies.

Two symptoms in io_uring's own source support the reading: `io_accept_prep()`
refuses multishot combined with a *specific* fixed slot, because one submission
producing unbounded installs is the case that forces kernel-managed allocation;
and `IORING_FILE_INDEX_ALLOC` still allocates at completion and reports
`-ENFILE` in `cqe->res`, at the one moment the submitter can do nothing about
it.

Corrected in the log and in `013` before this document was written.

**D-153 needs one addition: exit must be a wait, not a signal.** ◆ Closing an
io_uring instance cancels pending requests automatically — but until Axboe's
March 2025 series *"Cancel and wait for all requests on exit"*, those
cancellations happened **out of line**. Applications could not rely on ring exit
meaning the I/O had finished, and the concrete symptom in the cover letter was
**mount points staying busy after the test process had exited**.

Four lessons for a zombie that collects in-flight operations:

- **Exit must be a wait.** Six years of io_uring did it the other way and it
  was a bug.
- **Do not put a grace-period-based refcount on the teardown path** — the fix
  also replaced a `percpu_ref` with a plain atomic, because the RCU grace
  period was too slow there.
- **The zombie must hold every resource any in-flight operation can still
  touch.** The busy-mount symptom is what happens when it does not.
- **Uncancelable operations exist.** "Cancel is a request; exit is a wait" are
  different guarantees and only the second is total.

---

## 11. What this pass could not verify

Recorded because `010` set the precedent and because a research document that
claims no gaps is not one.

- **Whether VMware Workstation publishes an ACPI MCFG table.** No primary
  source. This is why CF8/CFC is the *safe* recommendation and not merely the
  simple one — treat ECAM as unavailable under VMware until measured.
- **VMware's actual `CAP.DSTRD`, `CAP.CQR`, `CAP.MQES`, `CAP.MPSMIN/MAX` and
  `CAP.CSS`.** Must be read at boot, never assumed. The queue-count and
  depth figures circulating for VMware's virtual NVMe are secondary sources.
- **Whether VMware places the NVMe function on bus 0 or behind a bridge.**
  Recursive enumeration makes the question go away, which is why it is
  recommended rather than answered.
- **Whether GRUB's C line counts translate to Rust.** The 673-versus-772
  numbers are measured; the inference that Rust preserves the ratio is not.
- **The backslash make code**, `0x5C` versus `0x5D` — see D-156. One source
  read one way, the table reads the other, and the code is deliberately left
  unanchored rather than guessed.
- **`lore.kernel.org` and `git.kernel.org` are behind an anti-bot wall.** The
  io_uring findings come from a mirror, GitHub, patchwork, LWN and man7, and
  rest on **v2 patch postings rather than merged commits**.
- **A negative cannot be proven on the io_uring claim.** No source ties direct
  descriptors to table exhaustion across LWN, two cover letters, three commit
  messages and the man page — but a list thread that was not read could say
  otherwise.
- **Zircon's monotonic clock path.** The no-writable-segment property is
  confirmed; whether `zx_clock_get_monotonic()` avoids a kernel entry is not.
- **Fuchsia's exact enforcement point for `ENUMERATE`** came through a
  summarising fetch of encoded source rather than raw text.
- **`wiki.osdev.org` returned 403 to every request** in two independent angles.
  Nothing above depends on it.
