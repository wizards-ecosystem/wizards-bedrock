# 014 — Storage (Phase 6)

The phase that gives the kernel something to wait for.

Phase 5 built a ring and gave it a caller that echoes a word back. Nothing in
this kernel has yet asked for something it could not have immediately: the
clock answers, `exit` retires, the doorbell serves. A `read()` from a disk is
the first operation whose honest answer is *not yet*, and the first whose
whole purpose is to hand back **authority** rather than bytes.

Both of those reshape the ABI, which is why D-149 puts them **before** the
filesystem rather than after it. Build `read()` synchronous and add the ring
form later and this kernel has two doors to one operation for ever — the
mistake Phase 5 avoided by freezing the handle table and the queue before
writing their callers.

So the order is: the ABI learns to carry a handle, to publish time without a
trap, and to survive a program that dies mid-operation. Then a disk, a format,
and a VFS built ring-native from the first byte.

This file is the phase's charter, written before its code per D-19. As with
`007`, `009` and `011`, each slice's design is recorded below **as it is
built**, so the file ends the phase as the description of what exists.

References: `012-research-notes-4.md` throughout — §1 (the format choice and
why the roadmap's premise was wrong), §2 (NVMe, and why virtio-blk is
disqualified), §3 (the ring parallel and why it is a trap), §4–§7 (rights, path
resolution, the seek position, the VFS shape), §8 (the async file path and how
few threads it needs), §9 (the time page), §10 (what the pass corrected).
`013-application-platform.md` is the argument for slices 1–3 and was written
first. Outside the project: the NVM Express Base Specification 2.2, Microsoft's
FAT32 specification, `fuchsia.io`'s rights model, and io_uring's cancellation
and teardown history.

---

## What earlier phases left owing

| Debt | Where recorded | What Phase 6 does |
|---|---|---|
| No completion carries a handle, so the ring is not the delegation path channels are | `011` close; D-151 | **Slice 1**, with the receiver reserving the slot at submission |
| Nothing blocks on a completion, so the wait queue is built and unused | `011` close | **Slice 1**: the first blocking read is what uses it |
| Port claims prove exclusivity, not device *entitlement* — the device half of the authority debt | D-77; `011` | **Slice 4**, with PCI discovery: a driver claims a device, not a port range |
| `sys_read`, `sys_write`, `sys_seek`, `sys_close`, `sys_open` are OWED or ABSENT | D-114; `userland/README.md` | Slices 6–8. `sys_open` stays ABSENT by design (C-3) and becomes `openat` from a directory handle |
| `sys_clock_get` is OWED over `libs/timebase` | D-114 | **Slice 2**: it stops being a call and becomes a load |
| An in-flight operation whose program retires is unspecified | D-153; `013` W-3 | **Slice 3** |
| `xtask/src/elf.rs` still transcribes ELF header offsets rather than using `libs/elf64` | D-105; `011` close | **Slice 9**, when the image builder is touched anyway |
| A wait with a timeout | `009` slice 4; `011` | Slice 1, where it falls out of the completion queue |
| Every new hardcoded window is interest on the KASLR debt | D-46 | Slices 2 and 4 each add an entry: the time page and the NVMe register window |

Not this phase's debt, recorded so nobody mistakes it: KASLR waits for Phase 12
(D-46); SMP bring-up waits for Phase 12, and with it the four races D-126,
D-129, D-141 and D-145 record as unreproducible (D-154); compat-fork waits for
Phase 9; WizFS waits for Phase 13 (D-73).

---

## Order of work (proposal)

The order is forced twice over. The ABI slices come first because a VFS is the
first caller that waits, and a caller freezes the interface it uses (D-149).
Within the storage half: nothing can read a format without a block device,
nothing can enumerate without a reader, and the VFS needs at least two backends
before its abstraction is anything but a guess.

| Slice | Contents | Status |
|---|---|---|
| 1 | **A completion carries a handle.** Reservation extended to handle-table slots, attenuation on arrival, and the first operation that genuinely waits | Done |
| 2 | **The time page.** Two regions — immutable constants, and a versioned clock with a revocation gate and a syscall fallback | |
| 3 | **Death mid-operation.** In-flight operations collected with the zombie; exit is a *wait*, not a signal | |
| 4 | **PCI, and a device that is claimed.** CF8/CFC enumeration recursing through bridges, a `Device` capability that supersedes bare port claims, BAR mapping uncacheable | |
| 5 | **NVMe, polled.** Twelve-step init, one I/O queue pair, `IEN=0`, PRP construction in `libs/`, and the `size − 1` capacity rule proved | |
| 6 | **FAT32, read-only, short names.** `libs/fat32` over a block trait, checked against the ESP `xtask` already builds | |
| 7 | **The VFS.** A directory trait — `lookup`, `enumerate`, `modify` — enum dispatch, and path resolution that cannot ascend | |
| 8 | **tmpfs and devfs**, so the abstraction has more than one backend and devfs proves that not everything is bytes | |
| 9 | **The file handle.** `READ`/`WRITE` on files, `TRAVERSE`/`ENUMERATE`/`MODIFY` on directories, offsets per operation, and a `read` that is a submission | |
| 10 | **Phase close.** The VMware boot (D-18, and the first one that runs the checks — D-135), the adversarial audit, the roadmap and this file's closing section | |

---

## The three ABI decisions, and what they cost in code

`013` is the argument; this is the shape.

### A completion carries a handle (D-151)

A submission declares whether it may produce a handle. If it does, admission
reserves **two** things: a completion slot, as every submission already does,
and a handle-table slot from the receiver's quota. Either unavailable and the
submission is refused *at submit* — which is the moment the program can do
something about it.

Three properties follow, and each is a check:

- **The reservation is returned on every terminal path.** A refused operation,
  a cancelled one, and a retired program's are three paths and the third is
  slice 3's. `012` §10 records `IORING_FILE_INDEX_ALLOC` reporting `-ENFILE` at
  completion as the symptom of getting this wrong.
- **Rights attenuate on arrival**, the way D-110 makes the channel path
  attenuate. Otherwise the ring becomes the widening path around that
  discipline, which is the one thing that would make it worse than the
  channels it is joining.
- **Exactly one terminal completion still holds.** Cancellation is itself an
  operation with its own reservation and its own completion, and never
  substitutes for the target's — `012` §8, and the antidote to the double-completion
  bug class that produced two io_uring CVEs.

### The time page is global and has two regions (D-152)

An **immutable constants region** — tick rate, page size, ABI version — with no
seqlock and no version counter, because there is nothing to tear. Zircon's
model, and it removes the most error-prone reads from the protocol entirely.

A **versioned clock region** — Linux's seqlock protocol verbatim, including
*both* memory barriers, `nsec` stored pre-shifted so a reader does one
multiply-add and no division, a `clock_mode` **revocation gate**, and a syscall
fallback that is always reachable.

**One question this phase must answer before writing the writer.** `012` §9:
Linux tolerates a naive reader spin only because its timekeeping update runs
with interrupts off. If this kernel's writer can be preempted while `seq` is
odd, readers spin unboundedly and the protocol is wrong for us. The writer runs
in the timer path; establishing that it cannot be preempted there is a
precondition, not a comment.

### An operation outlives its program, and exit waits (D-153, amended)

`LastRites`, `orphan` and `collect` already exist for a dead program's memory.
An operation in flight becomes one more thing the zombie holds — and `012` §10
adds what the original entry did not say: **exit must be a wait, not a signal.**

io_uring cancelled out of line for six years and the symptom was mount points
staying busy after the process had exited. Three consequences: collection waits
for the operation rather than marking it abandoned; the zombie holds every
resource any in-flight operation can still touch; and the teardown path gets no
grace-period-based refcount.

---

## The storage half

### The format: FAT32, read-only, short names first (`012` §1)

**The roadmap's premise was wrong and the choice is still FAT32.** Measured
against GRUB's own two readers — one project, one style, both read-only — FAT
is 673 lines to ext2's ~772. Thirteen per cent. Nobody should start this slice
believing FAT is dramatically simpler, because it is not, and the traps are
differently shaped rather than fewer.

What decides it is the **corpus**. `xtask` already builds a FAT32 ESP through
`fatfs`, and two images built seconds apart hash identically — with chrono off,
`fatfs` pins the volume ID and the timestamps. Meanwhile all three pure-Rust
ext2 image builders fail to run on this machine, and the reference read-only
ext2 crate generates its corpus by shelling out to `mkfs.ext2`, which D-16
forbids.

Three consequences for the slice:

- **The oracle is `fatfs` on the host.** Two independent implementations of one
  format, one of them mature, with the disagreement between them as the test.
  That is a better arrangement than a dependency.
- **Short names only, matching the corpus.** `xtask` does not enable `fatfs`'s
  `lfn` feature, so today's ESP is 8.3. Enabling it is a one-word change that
  adds long-name coverage when the reader is ready.
- **Synthesise inode numbers from directory-entry location**, which is what
  GRUB and Linux's `fs/fat` both do. FAT has no POSIX metadata and the VFS must
  not be shaped by its absence — WizFS arrives in Phase 13 and the VFS is
  designed against *its* semantics.

Do not implement exFAT. It is the one variant with live patents; FAT32
including long names is unencumbered, and Microsoft's licensing page no longer
lists a FAT programme.

**One weakness to budget for:** the corpus comes from a single writer. Obtain a
foreign-produced FAT32 image so the reader is not validated solely against the
tool that made its input.

### The device: NVMe, polled (`012` §2)

**virtio-blk is disqualified, not deprioritised.** VMware exposes no virtio
device of any kind, so under D-18 it could never close a phase — and none of
the virtqueue work carries to the driver that could. AHCI spans both hosts and
is a worse first driver for the same prerequisites. NVMe is the only overlap
with a freely published specification.

**Polled is what makes it affordable.** `Create I/O Completion Queue` with
`IEN=0` disables interrupts for that queue, which removes "this kernel has no
interrupt sharing" from the critical path entirely.

Four things that decide whether it works at all, all from the specification:

- **Bus Master Enable.** PCI Command register bit 3, alongside Memory Space
  Enable. Without it the controller cannot DMA, queues never advance, and
  **there is no error anywhere to read.** This belongs in a comment at the
  write site.
- **Enumeration must recurse through bridges.** VMware places functions behind
  `pciBridge0`–`pciBridge7`; a bus-0-only scan may not find the controller.
- **The BAR is mapped uncacheable**, and doorbells are write-only so the tail is
  shadowed in software. Mapping it write-back is a silent, hard-to-debug bug.
- **`CAP` is read, never assumed** — `MQES`, `DSTRD`, `CQR`, `MPSMIN`/`MPSMAX`,
  `TO`. `012` §11 records that VMware's values are unmeasured.

CF8/CFC rather than ECAM: it reaches everything a read-only driver needs, and
it is the only mechanism guaranteed present on **both** boot targets — QEMU's
i440fx publishes no MCFG and VMware emulates a 440BX. An MCFG path is purely
additive later.

### The ring parallel is a trap (`012` §3)

NVMe is a submission/completion ring with a doorbell and this kernel has one.
Two research angles independently concluded: **share the arithmetic, never the
structure.**

The invariant does generalise, and is worth stating once — Base 2.2 §3.3.1.2.1
puts the same obligation on the host that D-79 puts on the kernel: never admit
work whose completion has nowhere to go. But the completion tail *is not
visible to the host*; new entries are found only by phase tag; completions are
correlated by CID rather than slot; and **capacity is `size − 1`**, an
off-by-one whose symptom is a hang.

So: the queue-index and PRP arithmetic go to `libs/`, where `cargo test` and a
mutation reach them and where the `size − 1` rule is proved once. The byte
layout, phase tag, doorbell and MMIO stay in `wost`. **No shared `Ring<T>`.**

### The VFS: a directory trait, and enum dispatch (`012` §7)

Not Linux's shape. Its five operation tables and the dcache exist to serve a
global namespace with a mount tree and a shared lookup cache, neither of which
this kernel has. Importing the shape without the requirements is how a VFS
becomes premature.

**The abstraction is over directories**, because path resolution is the only
code that must be backend-agnostic: `lookup(name)`, `enumerate(cookie)`,
`modify`. That is 9P's walk/read/remove.

**Enum dispatch while all three backends are in-kernel** — monomorphic,
allocation-free, exhaustively checked when a fourth arrives. Promote to dynamic
dispatch only when a backend must be added without editing the enum, which here
means *when a filesystem lives in userspace*, and at that point the abstraction
is the protocol and the trait is irrelevant.

**devfs does not go through a byte interface.** A devfs entry has no bytes; it
is a name that yields a handle of another type. Forcing it to implement
`read_at` is the "everything is a file" conflation, and WASI's own README
rejects it on exactly these grounds. `lookup` on devfs returns a device handle.

### Rights, and the one split worth stealing (`012` §4)

The four universal rights are already, bit for bit, Zircon's basic rights. Per
type:

- **File:** `READ`, `WRITE`. `EXECUTE` is *reserved and not defined* — until the
  VFS can hand out an executable mapping of file bytes, the bit checks nothing
  and is a syscall name in disguise.
- **Directory:** `TRAVERSE`, `ENUMERATE`, `MODIFY`.

`TRAVERSE` versus `ENUMERATE` is the valuable one: "may resolve a name I know"
versus "may list the names", and a real kernel distinction by D-78's rule
because the resolver consults one and readdir the other. Capsicum folds
enumeration into read; WASI Preview 1 had both by accident of being
syscall-named.

**Rename and link split authority by side.** They are the only operations
taking two directory handles, and Capsicum's man page notes that
`CAP_LINKAT_SOURCE` "allows linking files with broader rights" — folding the
two into one bit is an **authority-laundering vector**.

**One set, monotone non-increasing.** A nested open may request the same rights
or fewer, never more. WASI Preview 1's base/inheriting pair forced its libc to
compute two masks per open and was deleted.

### Path resolution that cannot ascend (`012` §5)

1. **There is no `/`.** An absolute path is a *malformed argument*, not a
   permission failure — there is no root to resolve against.
2. **`..` is resolved lexically, in the kernel, before any node is touched.** A
   component vector that pops on `..` and errors on underflow means the
   resolver never holds a parent pointer, so **there is no operation that can
   walk upward**. That is stronger than `RESOLVE_BENEATH`, which enforces a
   predicate on a resolver still capable of ascending. Fuchsia reached the same
   answer — *Dot Dot Considered Harmful* — and moved it client-side; this kernel
   has no trusted client library, so it goes in the resolver.
3. **Walk node to node, holding a reference per step**, never re-deriving a node
   from a name. That is what closes the TOCTOU gap Linux needed `openat2` for.

The invariant, which is mutation-testable: **no sequence of path operations
from a directory handle can yield a node that is not a descendant of that
handle's node at some point during the call.**

Symlinks are deferred. If they arrive, they resolve inside the same component
loop with a bounded expansion counter.

### The seek position lives nowhere (`012` §6)

Offsets are per operation. No `seek`, no `tell`, no cursor on a handle.

The evidence is one-directional: WASI Preview 2 deleted it, 9P never had it,
Capsicum gates it as separate authority, and Fuchsia made it per-connection —
breaking POSIX `dup` semantics anyway. And it is right *here* for reasons
beyond precedent: a shared cursor is a lock on the hot path; it is the only
reason two handles to one file have observable coupling, which in a capability
system is a covert channel; and per-operation offsets are what a userspace
cursor is built *from*, while the reverse needs a kernel lock.

**The exception, honestly.** Directory enumeration cannot be stateless — entries
are variable-length and the set mutates. `read_dir` returns an opaque `u64`
cookie and takes it back, so the cursor lives in the caller's memory and the
kernel keeps **zero** position state on any handle.

### How few threads a blocking read needs (`012` §8)

io_uring's history is a march *away* from its worker pool, and its bounded pool
alone would be `4 × nr_cpus`. This kernel has sixteen thread slots total.

1. Always try inline with a nowait flag.
2. Anything pollable registers a wait — never a thread.
3. **A regular-file read that sleeps is the only case that genuinely needs
   one** — and if the block layer is submit-plus-completion, driving the read as
   a state machine may need **zero**. That is the direction Linux was
   travelling and could not finish, because it had to keep every existing
   filesystem working. This kernel has one.
4. If a worker is needed: one or two dedicated slots, a bounded queue, and
   **queue-full is a submission-time rejection**. That falls straight out of
   reserve-at-submission.
5. **Copy the submission entry at punt time.** The ring entry may be overwritten
   before the worker runs.

---

## How it will be checked

The three locations of `005` apply unchanged, and this phase leans hard on the
first:

| The thing under test | Where | How |
|---|---|---|
| PRP construction, queue indices, FAT cluster arithmetic, path component resolution | `libs/` | `cargo test`, doctests, mutation |
| A real read from a real device | `kernel/src/selftest.rs` | `cargo xtask test-boot` |
| What this repo says about itself | `xtask/` | `cargo test` |

Specific checks this phase owes, named now so they are not invented later:

- **The FAT reader against `fatfs`.** The host builds an image and reads it
  back with `fatfs`; the kernel reads the same image and must agree. Two
  implementations, one corpus, and the disagreement is the failure.
- **The `size − 1` capacity rule**, proved in `libs/` with a mutation, because
  getting it wrong hangs rather than errors.
- **A path that tries to ascend**, from every direction: absolute, `..` past
  the root, `..` in the middle that would escape, and a rename underneath a
  walk.
- **A handle arriving by completion with rights it was not offered**, which must
  be refused at the attenuation.
- **A program that dies with an operation outstanding**, and every resource
  back afterwards — frames, handles, reservations and the worker slot.
- **The seqlock's writer is not preemptible**, established rather than assumed.
- **The device claim supersedes the port claim**: two drivers cannot claim one
  device, and the refusal strands nothing (D-138).

---

## Known holes, carried deliberately

- **No write path.** The format is read-only and the phase is honest about
  that: "zero on-disk-write risk" is the roadmap's phrase and the point of
  choosing a third-party format at all.
- **No page cache.** A read goes to the device. Whether that is affordable is a
  measurement this phase should take rather than assume, and a cache is a
  decision for whoever has the numbers.
- **No symlinks, no hard links.** FAT has neither and the VFS should not grow
  them for one backend.
- **One namespace per program, and no way to compose two.** Plan 9's `bind` and
  Fuchsia's namespace construction are both out of scope; a program gets the
  directory handles it was given.
- **The corpus has one writer**, until a foreign image is obtained.
- **`sys_open` stays ABSENT** in the sysdep taxonomy. It becomes `openat` from
  a directory handle, which is what C-3 means and what `userland/README.md`
  already records.

---

## Decisions this phase must make (with their tensions)

Each is minted when it is taken, not batched — and the number is not reserved
here, because a citation to an entry that does not exist is what
`xtask/src/decisions.rs` refuses (D-124).

1. **Does a device claim replace a port claim, or sit above it?** D-77's device
   half is owed here. A `Device` capability that *contains* the port and MMIO
   claims is cleaner and makes the registry's permanence (D-138) a property of
   one object rather than of every range. A `Device` that merely *authorises*
   claiming leaves two mechanisms.
2. **Is the block layer a trait or a concrete NVMe?** One device today. `005`
   forbids the speculative API and `012` §7's argument about enum dispatch
   applies here too — but a `libs/fat32` that takes a block trait is testable on
   the host against a file, which is worth a great deal.
3. **Does a page cache exist in this phase?** Reading a directory means reading
   the same sectors repeatedly. The measurement decides it, and the measurement
   does not exist yet.
4. **What is a `File` handle's object?** An inode-like node the VFS owns, or a
   thinner thing that names (backend, node id). The second is smaller; the
   first is what a future write path and a future cache both want.
5. **Does `read` take a length, or a provided buffer?** `012` §8's warning
   applies: `-ENOBUFS` at completion is the same late-failure shape as the
   `-ENFILE` D-151 designed away. If lengths are always caller-supplied the
   question does not arise, and it should not arise unless something needs it.

---

## Slice 1 — A completion carries a handle

**Landed 2026-08-25.** D-163, D-164, D-165. 151 machine checks, 606 host tests,
33 mutations.

Three opcodes, and each exists to pay one of the debts `011` closed with:

| Opcode | What it proves |
|---|---|
| `Duplicate` | A completion carries **authority**, narrowed on arrival |
| `Timer` | An operation whose honest answer is *not yet*, and the first blocking wait |
| `Cancel` | An operation ends early **with its own completion**, never with the target's |

### What had to move first

Two things blocked the whole slice and neither was about the ring.

**A program's handle table was frozen the moment it was spawned.** `spawn`
built one, inserted the bootstrap channel, wrapped it in an `Arc` — and every
mutating method took `&mut self`. Wrapping it in a lock does not work: `insert`
takes the handle arena's leaf, so a table leaf held across it is the nesting
`lock_leaf` forbids, and a token-bearing lock cannot be taken in a handler at
all. The count became one `AtomicU64` instead and every method takes `&self`.
No lock added, no lock order changed (D-163).

**A ring-3 thread had no way to stop running.** `sleep_ns` needs a
`ThreadContext` and there is none inside a trap. `thread::sleep_from_trap` is
`retire_from_trap`'s shape without the retirement — mark the caller `Sleeping`,
hand the CPU on, and switch through the frame, which is what preemption already
does and the only place a switch is legal (D-164).

### The promise is a value, not a rule

D-151 asks that the reservation be returned on every terminal path and names
three. A property stated as a list of paths is a copy of the control flow, and
it stops being a copy the moment a path is added. So `reserve_slot` returns a
`SlotReservation` with a destructor: `install` consumes it, and every other
end — a refusal, a cancellation, a `?` on the way through, slice 3's dead
program — gives it back without being told to.

The arithmetic went to `libs/handle-space` as `Quota`, two counts in one word so
the promise-to-handle transition is a single compare-and-exchange. Its overflow
test found a real edge on the first run: `reserve` could carry a full promise
half into the held half. Not reachable through a quota of 64, which is exactly
why it had to be right.

### What the checks are, and the one that could not fail

Six checks, seven deliberate breaks, four mutations. The one worth recording is
the one that was wrong.

*A refused handle operation returns its promise* was written against a table
with room for exactly one handle. Every refusal therefore happened **at** the
reservation, no promise was ever made, and `promised() == 0` was true for the
wrong reason. Deleting the destructor left it green. It was found by D-28 and
only by D-28 — and the tell was that the break reddened a *different* check,
one that asserts the same thing in passing from a table with room. The check
about the property could not see it; the check that mentioned it could (D-165).

The rebuilt version reserves and then fails three times, at a different point
each: at the lookup, at the right, and at the attenuation. The full-table case
is a separate check now, because it is a separate claim — it is about *when*
the program is told, which is D-151's whole argument for reserving at all.

### What this slice deliberately did not do

- **`Pending` is a timer and nothing else.** D-41 forbids the abstraction with
  one implementation; the second kind arrives with the block device in slice 5.
- **No worker thread.** `012` §8 is emphatic, and a state machine driven from
  the waiting thread needs none. Sixteen thread slots is the reason and the
  measurement will be taken in slice 5.
- **`Queue`'s `WaitQueue` is still unused by ring 3**, and that is not an
  oversight: it takes a `ThreadContext`, so it is what a *kernel* thread waits
  on. A ring-3 thread waits through the scheduler.
