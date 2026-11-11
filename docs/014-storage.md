# 014 - Storage (Phase 6)

The phase that first gives the kernel something to keep in flight.

Phase 5 built a ring and gave it Echo. Phase 6 then separated four milestones
that the opening draft conflated: Timer is the first operation whose
answer is *not yet*; Duplicate is the first completion that carries derived
authority; OpenAt is the first namespace operation that produces authority;
and Read is the first storage/data caller that needs a handle, offset, buffer
and length together. D-200 later moved Read service to one bounded file worker,
so `RingEnter` now admits the operation and `RingWait` leaves through its saved
trap frame until the worker posts the terminal completion.

Those milestones reshape the ABI, which is why D-149 puts them **before** the
filesystem rather than after it. Build `read()` synchronous as a separate call
and add the ring form later and this kernel has two doors to one operation for
ever: the mistake Phase 5 avoided by freezing the handle table and the queue
before writing their callers.

This file was the phase's charter, written before its code per D-19, and
records each slice as it was built.

References: `012-research-notes-4.md` throughout: section 1 (the format choice and
why the roadmap's premise was wrong), section 2 (NVMe, and why virtio-blk is
disqualified), section 3 (the ring parallel and why it is a trap), section 4-section 7 (rights, path
resolution, the seek position, the VFS shape), section 8 (the async file path and how
few threads it needs), section 9 (the time page), section 10 (what the pass corrected).
`013-application-platform.md` is the argument for slices 1-3 and was written
first. Outside the project: the NVM Express Base Specification 2.2, Microsoft's
FAT32 specification, `fuchsia.io`'s rights model, and io_uring's cancellation
and teardown history.

---

## What earlier phases left owing

| Debt | Where recorded | Paid |
|---|---|---|
| No completion carries a handle, so the ring is not the delegation path channels are | `011` close; D-151 | Slice 1 (D-163), with the receiver reserving the slot at submission |
| Nothing blocks on a completion, so the wait queue is built and unused | `011` close | Slice 1 (D-164): Timer becomes the first deferred operation |
| Port claims prove exclusivity, not device *entitlement*: the device half of the authority debt | D-77; `011` | Slice 4 (D-168): a driver claims a device, not a port range |
| The five file sysdeps were OWED or ABSENT under D-114's former taxonomy | D-114; amended by D-181 | Slices 6-9 build the VFS, `OpenAt` and `Read`; the sysdeps remain incomplete, as the close records |
| `sys_clock_get` was OWED under that former taxonomy | D-114; amended by D-181 | Slice 2: monotonic time stopped being a call (D-166); D-181 records the reader and fallback as PARTIAL |
| An in-flight operation whose program retires is unspecified | D-153; `013` W-3 | Slice 3 (D-167) |
| `xtask/src/elf.rs` still transcribes ELF header offsets rather than using `libs/elf64` | D-105; `011` close | Program loading paid early in D-158; D-213 moved the remaining section-table file-header fields into `libs/elf64` |
| A wait with a timeout | `009` slice 4; `011` | Slice 1 (D-164) |
| Every new hardcoded window is interest on the KASLR debt | D-46 | Slice 2's time page; slice 4 needed none: the NVMe window is discovered, not fixed |

Not this phase's debt, recorded so nobody mistakes it: KASLR and SMP bring-up
wait for Phase 12 (D-46, D-154); the finite register owns their exact triggers.
Compat-fork waits for Phase 9; WizFS waits for Phase 13 (D-73).

---

## Slices

The order is forced twice over. The ABI slices come first because Timer needs
real deferral and Read is the first storage/data caller to freeze the widened
entry it uses (D-149).
Within the storage half: nothing can read a format without a block device,
nothing can enumerate without a reader, and the VFS needs at least two backends
before its abstraction is anything but a guess.

| Slice | Contents | Status |
|---|---|---|
| 1 | **A completion carries a handle.** Result-slot reservation, same-object attenuation, and the first operation that waits; cross-type derivation arrives with OpenAt in slice 9 | Done |
| 2 | **The time page.** Immutable constants, a versioned clock with a revocation gate, and proof that kernel Uptime survives revocation; D-181 records the still-incomplete shared userspace fallback | Done |
| 3 | **Death mid-operation.** In-flight operations collected with the zombie; exit is a *wait*, not a signal | Done |
| 4 | **PCI, and a device that is claimed.** CF8/CFC enumeration recursing through bridges, a `Device` capability that supersedes bare port claims, BAR mapping uncacheable | Done |
| 5 | **NVMe, polled.** Twelve-step init, one I/O queue pair, `IEN=0`, PRP construction in `libs/`, and the `size - 1` capacity rule proved | Done |
| 6 | **FAT32, read-only, short names.** `libs/fat32` over a block trait, checked against the ESP `xtask` already builds | Done |
| 7 | **The VFS.** A directory abstraction over resolve, read and enumerate (`modify` is reserved for a future write path), enum dispatch, and path resolution that cannot ascend | Done |
| 8 | **tmpfs and devfs**, so the abstraction has more than one backend and devfs proves that not everything is bytes | Done |
| 9 | **The file handle.** File `READ`, directory `TRAVERSE`/`ENUMERATE`, offsets per operation, and a `Read` submission later deferred by D-200 | Done |
| 10 | **Phase close.** The VMware boot (D-18, and the first one that runs the checks, D-135), the adversarial audit, the roadmap and this file's closing section | Planned slices complete. v0.2.0 qualified the diagnostic image (D-449); the foundation stays open on PW-006 and PW-010 |

Every slice landed 2026-08-25; slice 10's decisions are D-174 to D-179.

---

## The ABI half

`013` is the argument; this is the shape.

### A completion carries a handle (D-151; slice 1, D-163 to D-165)

Three opcodes, each paying one of the debts `011` closed with: `Duplicate`
proves a completion carries **authority**, narrowed on arrival; `Timer` is an
operation whose answer is *not yet*, and the first blocking wait; `Cancel` ends
an operation early **with its own completion**, never with the target's.

A submission declares whether it may produce a handle. If it does, admission
reserves **two** things: a completion slot, as every submission already does,
and a handle-table slot from the receiver's quota. Either unavailable and the
submission is refused *at submit*, which is the moment the program can do
something about it.

- **The reservation is returned on every terminal path.** `012` section 10 records
  `IORING_FILE_INDEX_ALLOC` reporting `-ENFILE` at completion as the symptom of
  getting this wrong. A property stated as a list of paths is a copy of the
  control flow, so `reserve_slot` returns a `SlotReservation` with a
  destructor: `install` consumes it, and every other end (a refusal, a
  cancellation, a `?` on the way through, a dead program) gives it back. The
  arithmetic is `libs/handle-space`'s `Quota`, two counts in one word so the
  promise-to-handle transition is a single compare-and-exchange.
- **Same-object rights attenuate on arrival**, the same non-escalation rule a
  future channel transfer must obey. A cross-type producer may derive only a type-specific
  right authorised by the source operation and preserves no universal right
  the source lacked; OpenAt's `TRAVERSE`-to-`READ` rule is the worked case.
- **Exactly one terminal completion still holds.** Cancellation is itself an
  operation with its own reservation and its own completion, and never
  substitutes for the target's: `012` section 8, and the antidote to the double-completion
  bug class that produced two io_uring CVEs.

A program's handle table count is one `AtomicU64` and every method takes
`&self`, because a table lock held across `insert` would nest the arena's leaf,
which `lock_leaf` forbids (D-163). A ring-3 thread stops running through
`thread::sleep_from_trap`, which marks the caller `Sleeping` and switches
through the frame, the only place a switch is legal (D-164). A check must
reach the code it is about (D-165): *a refused handle operation returns its
promise* first ran against a table with room for one handle, so no promise was
ever made and deleting the destructor left it green.

At this slice `Pending` was a timer and nothing else (D-41 forbade the
abstraction with one implementation); D-200 later added queued and running Read
kinds when deferred device service arrived. `Queue`'s `WaitQueue` is unused by
ring 3 because it takes a `ThreadContext`: it is what a *kernel* thread waits
on, and a ring-3 thread waits through the scheduler.

### The time page is global and has two regions (D-152; slice 2, D-166)

One page is read-only in every program at `0x0000_1000_0000_0000`.
An **immutable constants region** (tick rate, page size, ABI version) with no
seqlock and no version counter, because there is nothing to tear. Zircon's
model, and it removes the most error-prone reads from the protocol entirely.

A **versioned clock region**: Linux's seqlock protocol with both barriers,
whole `base_ns` plus a fractional carry, a 128-bit multiply/shift in the
reader, and a `clock_mode` **revocation gate**. A kernel check proves ring
Uptime still answers after revocation. The shared userspace client that can
correlate its fallback completion remains old Phase 7 work (D-181).

The base is split because `base_ns << shift` overflows a `u64` after about
eight seconds of uptime at a 1 GHz counter's shift, and the fraction matters: a
clock updated at a kilohertz that dropped it falls a microsecond behind every
second. Linux tolerates a naive reader spin only because its timekeeping update
runs with interrupts off (`012` section 9), so the writer's non-preemptibility
is a precondition, not a comment: the guard closes the window, and the writer
reads its own odd sequence value back, because on one CPU nothing else can witness it
(D-154's class).

The page exists even when the clock does not: it is built at boot whatever the
clock did, `mode` is `MODE_NONE`, and a spawn that cannot show it is refused.
*The time page and the kernel clock tell the same time* is a sandwich, the
page's reading taken between two `now_ns` reads, with no tolerance for an
off-by-one to hide in.

**This clock makes no periodic adjustment, and that limit is explicit.**
`time::init` normally accepts two believable PIT windows; a complete CPUID
0x15 ratio on an invariant TSC can supply the rate when those windows refuse,
while leaf 0x16 stays diagnostic. D-225 retains the claimed PIT to validate
every bounded APIC appointment. Two consecutive conclusive APIC/TSC
disagreements freeze public time, revoke the page and unwind every Timer; late
or folded windows are inconclusive. PIT samples continue a private scheduler
epoch so an already-running program remains preemptible without publishing
untrusted elapsed time. Suspend and NTP adjustment remain unsupported.

`WIZ_CALL_CLOCK_GET` is **deleted**, and its absence is the point.
`WIZ_CALL_RING_WAIT` takes 3; D-181 later deleted the unimplemented owed
numbers. The C header carries the page's address, version, modes and every
field offset, and the cross-check compares all of them against `timepage.rs`:
a `mode` read from the wrong place is a clock that reads as usable when it is
not.

### An operation outlives its program, and exit waits (D-153, amended; slice 3, D-167)

`orphan` and `collect` already exist for a dead program's memory; `LastRites`
is now only a self-test snapshot (D-248). An operation in flight becomes one
more thing the production zombie holds, and `012` section 10 adds what the original
entry did not say: **exit must be a wait, not a signal.**

The hazard: a retired program's ring pages go back to the allocator in
`reclaim`, and an operation in flight will later write a completion into a page
that is about to be somebody else's. So the ring is quietened first: `abandon`
ends every operation and posts the completion it was owed, into pages the
zombie still holds, because posting is what spends the reservations, and a
reservation never spent is a ring whose accounting says work is outstanding for
ever. Collection
waits for the operation rather than marking it abandoned; the zombie holds
every resource any in-flight operation can still touch; and the teardown path
gets **no grace-period-based refcount**, the out-of-line cancellation that took
io_uring six years to undo.

A promise holds an `Arc` to its table, so a program that dies holding one
leaves the table alive exactly as long as the promise; the check then requires
the table's `Weak` to stop upgrading, because a table nothing will ever free is
the other failure. A polled NVMe read cannot be ended by
writing a completion, because the device has the command. `abandon` ends only
cancellable entries and returns; the caller keeps the owner and retries, and teardown
releases nothing until `is_quiet`.

---

## The storage half

### The format: FAT32, read-only, short names first (`012` section 1; slice 6, D-170)

**The roadmap's premise was wrong and the choice is still FAT32.** Measured
against GRUB's own two readers (one project, one style, both read-only), FAT
is 673 lines to ext2's ~772: the traps are differently shaped rather than
fewer. What decides it is the **corpus**: `xtask` already builds a FAT32 ESP
through `fatfs` reproducibly, while all three pure-Rust ext2 image builders
fail to run on this machine and the reference read-only ext2 crate shells out
to `mkfs.ext2`, which D-16 forbids.

- **The oracle is `fatfs` on the host.** `xtask` builds one corpus with `fatfs`
  and it goes onto the NVMe namespace at sector 2048 and into a host test that
  reads it with **both** `fatfs` and `libs/fat32`. The disagreement is the
  failure, which beats depending on the mature one, because a dependency cannot
  disagree with itself.
- **The block layer is a trait, and it belongs to the reader.** `libs/fat32`
  defines `Sectors` because that is the parameter it needs; the host backs it
  with a byte slice, the kernel with the controller, and the same reader runs
  over both.
- **Short names only, matching the corpus.** `xtask` does not enable `fatfs`'s
  `lfn` feature, so the boot ESP is 8.3.
- **Synthesise inode numbers from directory-entry location**, which is what
  GRUB and Linux's `fs/fat` both do. FAT has no POSIX metadata and the VFS must
  not be shaped by its absence: WizFS arrives in Phase 13 and the VFS is
  designed against *its* semantics.
- 512-byte sectors only, refused rather than misread: that is what lets every
  buffer be a fixed array.

Do not implement exFAT. It is the one variant with live patents; FAT32
including long names is unencumbered, and Microsoft's licensing page no longer
lists a FAT programme.

**The formerly budgeted one-writer weakness is closed.** D-207 pins a volume
formatted and populated by `dosfstools`/`mtools`, with adversarial valid layouts
which both host readers must interpret identically. The local `fatfs` corpus
remains the one the kernel boots.

### The device: PCI and NVMe, polled (`012` section 2; slices 4 and 5, D-168, D-169)

**virtio-blk is disqualified, not deprioritised.** VMware exposes no virtio
device of any kind, so under D-18 it could never close a phase. AHCI spans both
hosts and is a worse first driver for the same prerequisites. NVMe is the only
overlap with a freely published specification.

**A driver claims a function, and the claim carries its windows** (D-168). A
port range has no identity and cannot be asked what it is, so a device claim
*contains* the port claim rather than authorising one: two mechanisms with a
gap between them is how a driver comes to hold a device capability and ports
that have nothing to do with it. The `Device` is a **kernel-side capability,
not a handle**, obtainable only from the registry, exactly as `Port<T>` is.
Slice 8 makes it resolvable internally through devfs but does not create a
handle. D-184 classifies a user-visible Device handle after the compiler gate;
it is not pre-Wizzy foundation work.

**CF8/CFC rather than ECAM**: it reaches everything a read-only driver needs,
and it is the only mechanism on **both** boot targets: QEMU's i440fx publishes
no MCFG and VMware emulates a 440BX. An MCFG path is purely additive later.
**Enumeration recurses through bridges**, because VMware places functions
behind `pciBridge0`-`pciBridge7`; the check machine is given a bridge and the
survey counts buses, not only functions. A bridge has two base address
registers, not six: its bus numbers sit where a device's third would be.
Window sizing is two functions, because a width you must remember to pass
correctly is a width that will be passed wrongly, and `pci::init` claims ports
outside its own leaf guard.

**Polled is what makes it affordable.** `Create I/O Completion Queue` with
`IEN=0` disables interrupts for that queue, which removes "this kernel has no
interrupt sharing" from the critical path entirely. Four things decide whether
it works at all, all from the specification:

- **Bus Master Enable.** PCI Command register bit 3, alongside Memory Space
  Enable. Without it the controller cannot DMA, queues never advance, and
  **there is no error anywhere to read.**
- **Enumeration must recurse through bridges**, as above.
- **The BAR is mapped uncacheable**, and doorbells are write-only so the tail is
  shadowed in software. Mapping it write-back is a silent, hard-to-debug bug.
- **`CAP` is read, never assumed**: `MQES`, `DSTRD`, `CQR`, `MPSMIN`/`MPSMAX`,
  `TO`. `012` section 11 records that VMware's values are unmeasured.

DMA publication and hardware retirement are separate authorities (D-251).
`libs/nvme::DmaState` can prove only reservation, doorbell publication,
matching completion and closure. After an ambiguous exit, the kernel may
recycle memory only through a linear stop token borrowing that controller's
register mapping and DMA state after fresh reads prove both `CC.EN = 0` and
`CSTS.RDY = 0`; otherwise queues and the current buffer remain retained.
Preparation itself is split into capability, disable, allocation, programming
and enable stages that carry one linear owner. A self-test meter paints the
dead boot stack when each storage candidate starts and scans it when the
candidate settles, so frames that never reach a register access count as well,
and it refuses a scan that saw less than the deepest live register-access
pointer. Across both candidates the painted high-water mark must leave two
pages of the guarded 32 KiB boot stack free. Historical measurements used
23,792 bytes in debug and 13,240 in release (D-287, D-326, D-359, D-361);
they do not qualify the changed DMA-owner layout. Its first debug measurement
used 25,072 bytes and failed the unchanged bound. D-433's compact mapping
identity reduced the next debug run to 24,432 bytes, leaving 8,336 free;
release and integrated qualification remain required. A frame below
preparation's register accesses, which the leaf alone never saw, fails only the named machine
check, as do the calibrated live-frame defects. An individual function-frame
ceiling could not have detected the original nested overflow.

### Slice 5: NVMe, polled (D-169)

The read checks write a distinct pattern into every block and read four,
because a disk of zeros proves nothing, and require the bytes **after** one
block to stay zero, because the extent of a read is only observable past its
end. *A register file the cache may answer for* was reported red when slice 5
landed and was a build failure; the phase close's mutation sweep found it
(D-175). The property is verified now, measured at the page-table entry: the
device window is `0x...0073` normally and `0x...0063` with the mutation, and
bit 4 is exactly `PCD`.

A read still spins, bounded rather than endless. Slice 9 gave it a submission
shape; D-200 later moved the polling to a preemptible worker and coupled its
queued/running state to slice 3's teardown rule. PW-704 keeps interrupt-driven
service stopped until a measured storage workload justifies it.

### The ring parallel is a trap (`012` section 3)

NVMe is a submission/completion ring with a doorbell and this kernel has one:
**share the arithmetic, never the structure.** Base 2.2 section 3.3.1.2.1
puts the same obligation on the host that D-79 puts on the kernel: never admit
work whose completion has nowhere to go. But the completion tail *is not
visible to the host*; new entries are found only by phase tag; completions are
correlated by CID rather than slot; and **capacity is `size - 1`**, an
off-by-one whose symptom is a hang.

So: the queue-index and PRP arithmetic go to `libs/nvme`, where `cargo test`
and a mutation reach them and where the `size - 1` rule is proved once. The
byte layout, phase tag, doorbell and MMIO stay in `wost`. **No shared
`Ring<T>`.**

### The VFS: a directory abstraction, and enum dispatch (`012` section 7; slices 7 and 8, D-171, D-172)

Not Linux's shape. Its five operation tables and the dcache exist to serve a
global namespace with a mount tree and a shared lookup cache, neither of which
this kernel has.

**The abstraction is over directories**, because path resolution is the code
that must be backend-agnostic. The delivered surface resolves, reads and
enumerates through enum dispatch; mutation waits for a write path. It is not a
trait pretending a backend can be added without editing the dispatcher.

**Enum dispatch while all three backends are in-kernel**: monomorphic,
allocation-free, exhaustively checked when a fourth arrives. Promote to dynamic
dispatch only when a backend must be added without editing the enum, which here
means *when a filesystem lives in userspace*, and at that point the abstraction
is the protocol and the trait is irrelevant.

**tmpfs** has no clusters, no chains and no device; its directories are a flat
table with parent indices. If the three operations fit both it and FAT32, they
are about directories rather than about FAT, which is the claim slice 8 exists
to make.

**devfs does not go through a byte interface.** A devfs entry has no bytes; it
resolves internally as `Node::Device`, and `vfs::read` takes a `File`, so
**there is no way to pass a device to it**: not a refusal at run time, an
absence at compile time. Forcing it to implement `read_at` is the
"everything is a file" conflation, and WASI's own README rejects it on
these grounds. OpenAt currently refuses that node; a user-visible `Device`
handle remains old Phase 7 work.

`with_controller` holds the controller's non-reentrant `ThreadLock` across an
enumeration callback (D-200), so a caller collects listed names and resolves
them afterwards, which is also the shape a caller must use: the type prevents a thread lock from surviving a scheduler
handoff; it cannot make callback re-entry succeed.

### Rights, and the one split worth stealing (`012` section 4)

The four universal rights are already, bit for bit, Zircon's basic rights. Per
type:

- **Channel:** no type-specific rights while it is only an opaque bootstrap
  identity (D-248).
- **File:** `READ`. Raw `WRITE`, `EXECUTE` and `MAP` are defined by
  `handle-space`, but no current object admits them.
- **Directory:** `TRAVERSE`, `ENUMERATE`. Raw `MODIFY` is reserved but no
  current Directory admits it while the namespace has no mutating operation.

`TRAVERSE` versus `ENUMERATE` is the valuable one: "may resolve a name I know"
versus "may list the names", and a real kernel distinction by D-78's rule
because the resolver consults one and readdir the other; the check requires
them to agree.

**Rename and link split authority by side.** They are the only operations
taking two directory handles, and Capsicum's man page notes that
`CAP_LINKAT_SOURCE` "allows linking files with broader rights", folding the
two into one bit is an **authority-laundering vector**.

**`OpenAt`'s rights are derived, never asked for** (D-173): one set, with
same-type authority monotone non-increasing, and no requested mask. A
directory preserves its parent's rights unchanged; a file preserves only
universal rights already held and derives `READ` from `TRAVERSE`. That
cross-type derivation is not a bitset subset, but it cannot recreate a
universal right or grant an unauthorised file operation. WASI Preview 1's
base/inheriting pair forced its libc to compute two masks per open and was
deleted. `WRITE` is never
granted, because there is no write path and a right that grants nothing is a
syscall name in disguise.

### Path resolution that cannot ascend (`012` section 5)

1. **There is no `/`.** An absolute path is a *malformed argument*, not a
   permission failure: there is no root to resolve against.
2. **`..` is resolved lexically, in the kernel, before any node is touched.**
   `libs/path-walk` pops on `..` and refuses underflow, so the resolver never
   holds a parent pointer and **there is no operation that can walk upward**:
   no parent pointer anywhere in `vfs.rs`. That is stronger than
   `RESOLVE_BENEATH`, which enforces a predicate on a resolver still capable of
   ascending. Fuchsia reached the same answer (*Dot Dot Considered Harmful*)
   and moved it client-side; this kernel has no trusted client library, so it
   goes in the resolver.
3. **Walk node to node, holding a reference per step**, never re-deriving a node
   from a name. That is what closes the TOCTOU gap Linux needed `openat2` for.

The invariant, which is mutation-testable: **no sequence of path operations
from a directory handle can yield a node that is not a descendant of that
handle's node at some point during the call.** It is exercised on **every**
backend, because it is a property of the resolver. A directory handle names its
own backend: each name must resolve where it belongs *and* nowhere else.

An escaping `..` is malformed because there is no parent, as an absolute path
is because there is no root.

Neither refusal is a denial: reporting either as a permission failure would
suggest some other program could. And nothing is clamped: an escape is refused
rather than resolved to the starting directory, a long name refused rather than
truncated, a deep path refused rather than cut short, because each would
silently answer a different question than the caller asked.

Symlinks are deferred. If they arrive, they resolve inside the same component
loop with a bounded expansion counter.

### The seek position lives nowhere (`012` section 6)

Offsets are per operation. No `seek`, no `tell`, no cursor on a handle. A
shared cursor is a lock on the hot path; it is the only reason two handles to
one file have observable coupling, which in a capability system is a covert
channel; and per-operation offsets are what a userspace cursor is built
*from*, while the reverse needs a kernel lock. WASI Preview 2 deleted it and 9P
never had it.

**The exception.** Directory enumeration cannot be stateless: entries
are variable-length and the set mutates. `read_dir` returns an opaque `u64`
cookie and takes it back, so the cursor lives in the caller's memory and the
kernel keeps **zero** position state on any handle.

### The file handle and the read (slice 9, D-173)

A `File` handle's object is the thinner one: a backend and a node within it,
not an inode the VFS owns. **`read` takes a provided buffer, and the buffer is
part of the ring**: a third shared page is the data area, so it is always
mapped, always the right size, and its bounds are two comparisons. There is no
user pointer to validate, no SMAP window to open and no fault to recover. The
cost is one copy: zero-copy needs memory objects, which is a later phase.

The entry is thirty-two bytes, because sixteen could not hold a handle, an
offset, a buffer and a length. D-181 repaired the four transcriptions of its
layout: one kernel offset map now feeds parser, serializer and helpers; the
check derives C offsets, widths and sizes, then checks actual emitted-ELF
operands plus entry base and count. A persistent `argument`-offset mutation
must make the program-ring check red.

The buffer bound is checked at submission, not after the read, where it would
be `-ENOBUFS` with the work already done. The controller's command identifier
makes "the device was not asked" a number the check holds unchanged.

**A ring-3 program still cannot reach any of this.** `spawn` gives a program
exactly one handle (its opaque bootstrap-channel identity), and there is no
ring-3 channel-receive operation yet. Giving a program a second handle at spawn
would break the invariant that makes pillar 1 checkable, so it waits for a
concrete channel protocol rather than for a convenience (D-248).

### How few threads eventual deferred reads need (`012` section 8)

io_uring's history is a march *away* from its worker pool, and its bounded pool
alone would be `4 x nr_cpus`. This kernel has sixteen thread slots total.

1. Always try inline with a nowait flag.
2. Anything pollable registers a wait, never a thread.
3. **A regular-file read that sleeps is the only case that needs
   one**, and if the block layer is submit-plus-completion, driving the read as
   a state machine may need **zero**.
4. If a worker is needed: one or two dedicated slots, a bounded queue, and
   **queue-full is a submission-time rejection**. That falls straight out of
   reserve-at-submission.
5. **Copy the submission entry at punt time.** The ring entry may be overwritten
   before the worker runs.

---

## How it is checked

The three locations of `005` apply unchanged, and this phase leans hard on the
first:

| The thing under test | Where | How |
|---|---|---|
| PRP construction, queue indices, FAT cluster arithmetic, path component resolution | `libs/` | `cargo test`, doctests, mutation |
| A real read from a real device | `kernel/src/selftest.rs` | `cargo xtask test-boot` |
| What this repo says about itself | `xtask/` | `cargo test` |

The phase's named checks: the FAT reader against `fatfs`; the `size - 1`
capacity rule in `libs/` with a mutation, because getting it wrong hangs rather
than errors; a path that tries to ascend from every direction (absolute, `..`
past the root, `..` in the middle, a rename underneath a walk); a same-object
Duplicate with rights it was not offered, and an OpenAt derivation that must
not restore universal rights; a program that dies with an operation
outstanding, with frames, handles, reservations and the worker slot back
afterwards; the seqlock writer's non-preemptibility; and a device claim that
supersedes the port claim, where two drivers cannot claim one device and the
refusal strands nothing (D-138). Inputs are chosen so the answers differ: the
corpus deletes a file to leave an `0xE5` slot, offers a whole sector of buffer
for a file that is not one, and holds `MOTD` twice in different directories.

---

## Known holes, carried deliberately

- **No write path.** The format is read-only:
  "zero on-disk-write risk" is the roadmap's phrase and the point of
  choosing a third-party format at all.
- **No page cache.** A read goes to the device. Whether reading the same
  sectors repeatedly is affordable is a measurement, the measurement does not
  exist, and a cache is a decision for whoever has the numbers. The volume is
  mounted per call for the same reason.
- **No symlinks, no hard links.** FAT has neither and the VFS should not grow
  them for one backend.
- **One namespace per program, and no way to compose two.** Plan 9's `bind` and
  Fuchsia's namespace construction are both out of scope; a program gets the
  directory handles it was given.
- **POSIX `sys_open` remains partial platform and compatibility work.** Native
  OpenAt resolves existing paths from a directory handle, but ring 3 cannot yet
  receive the first one. The adapter then supplies root/cwd/descriptor state
  and returns EROFS for creation, truncation and write modes rather than
  inventing a write path (C-3, D-181).

---

## Decisions this phase had to make

Each was minted when it was taken, not batched, because a citation to an entry
that does not exist is what `xtask/src/decisions.rs` refuses (D-124).

1. **Does a device claim replace a port claim, or sit above it?** A `Device`
   that *contains* the port and MMIO claims makes the registry's permanence
   (D-138) a property of one object; one that merely *authorises* claiming
   leaves two mechanisms. **It contains it (D-168).**
2. **Is the block layer a trait or a concrete NVMe?** One device, but a
   `libs/fat32` over a block trait is testable on the host against a file.
   **A trait, owned by the reader (D-170).**
3. **Does a page cache exist in this phase?** **No (D-170)**: the measurement
   does not exist.
4. **What is a `File` handle's object?** An inode-like node the VFS owns, or a
   thinner thing that names (backend, node id). **The thinner one (D-173)**;
   the fatter one is what a write path and a page cache both want, and this
   phase has neither.
5. **Does `read` take a length, or a provided buffer?** `012` section 8's
   warning applies: `-ENOBUFS` at completion is the same late-failure shape as
   the `-ENFILE` D-151 designed away. **A provided buffer, part of the ring
   (D-173).**

---

## Slice 10: Phase close

**2026-08-25.** D-174-D-179.

### What the phase is

A ring operation resolves a path from a directory handle, gets back a file
handle carrying derived, non-escalating authority, and reads bytes through the
NVMe device interface into a page it shares with the caller. QEMU and VMware
exercise virtual controllers; real hardware remains Phase 12. Every part of that
sentence was built by a different slice, and the order was forced: the ABI
first, because a caller freezes the interface it uses.

| | |
|---|---|
| Machine checks | 296 |
| Mutations | 2447 registered; complete-sweep verdicts live in source-bound qualification records |
| New `libs/` crates | `nvme`, `pci-config`, `fat32`, `path-walk` |
| Decisions | Phase decisions D-163 ... D-179; post-phase corrections D-181, D-182 and D-185 ... D-452 |

The two counts are **checked against the source at every commit**
(`xtask/src/prose.rs`), but mutation registration is not a current full-sweep
verdict. Source-bound qualification records carry those outcomes; this table
keeps only the registry's checked count. A number copied anywhere else can
disagree with both, which the per-slice tallies this file once carried did, in
seven places (D-175). The host-test count is deliberately absent: it changes
with every `libs/` crate and nothing would be learned from pinning it here.

The five file sysdeps remain incomplete for distinguishable reasons.
`sys_read` has a native explicit-offset Read, but needs a reachable file
handle and the compatibility layer's shared cursor. `sys_write` has no write
path. `sys_seek` changes that same POSIX cursor, which the native ABI
deliberately omits. `sys_close` has no current user route or duplicate
HandleTable-level mechanism; D-248 removed the uncalled shortcut so its
eventual operation must own safe last-reference destruction. `sys_open` layers
root/cwd/descriptor policy and explicit
EROFS refusals over read-only OpenAt, but also lacks the ring-3 route that
delivers its first directory. D-181's exact register, narrowed by D-182, keeps
those routes separate.

### What the audit found

One defect in three places, and a claim nothing checked (D-174). Three words a
submission carries (a rights word, a file offset, a cancellation token) were
reaching the kernel through `as` casts, so a program was answered about
something other than what it asked for. And `devfs::DEVICES` documented a test
that did not exist.

The rule for next time: **every field a submission carries is a value ring 3
chose.** `as` is not a conversion at that boundary.

### The VMware boot, which is not yet done

D-18 makes a VMware boot the definition of done, and D-135 said this would be
the first phase close whose VMware boot runs the checks rather than only
reaching a prompt. It ran them, and it came back `verdict v1 FAIL passed=117
failed=63`. The causes, each fixed in the tree and none a defect in what the
phase built:

- **The VM had no NVMe controller** (D-176). `cargo xtask vmware` now builds a
  second VMDK over the same namespace image QEMU is given, and a host test
  reads the template for the controller so this cannot revert quietly.
- **The clock refused to calibrate and did not say which guard refused**
  (D-176). The warning now names the guard and prints the two rates the window
  measured, believed or not. D-438 corrects a later diagnostic overstatement:
  failure to admit an agreeing believable pair does not prove that every window
  was individually believable or that two usable windows disagreed. A refused
  clock also makes the delayed storage-input self-test fail without blocking on
  missing keyboard events; its command hold is released before completion
  cleanup on every result.
- **Four paging checks had never run** (D-177). At `passed=172 failed=9`, four
  named physical `0x1000`, a page QEMU describes as memory and VMware does not.
  Each returned `false` out of its own setup, the value the harness also gets
  when a property is violated. They cut their pages from a run the map offers
  now, and all four pass on VMware.
- **VMware reports its keyboard translation bit clear and translates anyway**
  (D-178). At `passed=177 failed=5` the identity probe answered `(0xAB, 0x41)`,
  an MF2 keyboard behind a translating controller, where QEMU answers
  `(0xAB, 0x83)`. Two checks on the same boot disagreed, and the one that could
  not be lied to was right. **Set 1 is decoded now** (D-179) by a second table
  and a second machine sharing the key identities, the shift state and the
  layout; `both_sets_name_the_same_keys` fails if either drops a key, shadows an
  arm, or files one in the wrong half, and the gate makes QEMU translate so the
  set 1 path runs without a human.

**The 2026-08-25 image stood at `passed=182 failed=5`, and all five needed a
human**: the two injection rounds (a keystroke cannot be injected into a
headless VMware run the way QMP injects one into QEMU) and the three checks
that read what the first round delivered. D-181 and D-182 post-date that
measurement, so the current self-test image still needs the same human round;
the old total is evidence about that dated image, not a current verdict.

**So the planned slices are complete, while the finite foundation and its
human VMware close remain open.** The count in the table above is QEMU's. This
section records what VMware actually said rather than a boot that was not got,
which is what D-77 forbade, and what `012` section 11's "VMware's values are
unmeasured" already admits about the neighbouring number.

One thing stays unexplained on purpose: the clock refused to calibrate on the
first of these boots and calibrated on the five that followed, with no change
to the calibration path between them. That is why D-176's diagnosis prints the
guard and the two measured rates rather than a verdict.

**A second machine does not re-run the checks; it discovers which of them were
about the first machine.**

### What this phase taught the next one

D-155's rule (choose inputs where the answers differ) arrived from four
directions in ten slices: an input, a corpus of disk blocks, a directory tree,
and a rights word whose low half was valid. D-165's (a check must reach the
code it is about) arrived three times. Both are in `005` now, and both were
found by breaking checks rather than by reading them.

---

## DMA isolation through the IOMMU (post-phase)

**2026-09-29.** D-357; the programming choices are D-358 (2026-10-01), with
live-revocation ordering closed by D-383 (2026-10-10).

The storage controller is a bus-mastering PCI device, so the page tables do
not constrain it. Every DMA safety property this phase built is a property of
*when* a frame is device-owned (PW-200, PW-206, PW-213, D-339): a segment's
bytes return to the host only after a matching completion or a proven
controller reset. None constrains *which* physical addresses the controller may
name, so a wedged, buggy or hostile controller could DMA to any frame. The
IOMMU (Intel VT-d) constrains the reach: a second address-translation unit
between every PCI function and memory that checks each DMA access against
tables the kernel owns, and faults anything they do not map.

### The model: identity-mapped, scoped to live segments

The controller keeps naming physical addresses, and the IOMMU's second-level
tables map only the frames a live `DmaSegment` currently owns, so IOVA equals
host physical for a mapped frame and every other address faults. The queues and
each in-flight read or write buffer are mapped before the frame is handed to the
device and unmapped only after the existing lifecycle accepts its completion.
Preflight admits this register-based invalidation path only for VT-d major
versions 1 through 5. Unmap clears the leaves, performs any required write-buffer
flush, then completes a global IOTLB invalidation with both read- and
write-draining before the frame can return to the allocator. Completion is
accepted only when the hardware's actual-granularity field reports global, not
merely when its command bit clears. A hostile device's early completion therefore
cannot leave an older DMA request racing a new frame owner. The set the device can reach is
exactly the set the driver has published and not yet retired, which is the
property the software lifecycle already maintains, now enforced by hardware
rather than trusted of the device.

Per-segment address translation, where the IOMMU hands the device an address
unequal to the physical one and hides the memory map, is a strengthening this
does not need: identity-mapped-and-scoped already faults every access outside the
live set, which is the reach guarantee. It is a trigger row (PW-712) rather than
a shortcut taken here.

The unit is driven through its register-based invalidation interface, with
queued invalidation left off outside the private interrupt-cache transaction,
and a unit whose page walks bypass the processor caches (QEMU's reports exactly
that) has every table line flushed before the unit may depend on it. Before PCI
topology discovery, the kernel accepts only one RMRR-free segment-zero unit
whose static whole-segment authority is either `INCLUDE_PCI_ALL` or an exact
unique `WZDM` corroboration, and only with firmware TE, QIE, IRE and CFI already
inactive. It installs an
empty root, enables TE and completes a read-and-write drain before publishing
the boundary. A selected controller later gains only its exact context after it
is prepared and verified disabled; the unit lives in one kernel-owned slot that
every DMA seam locks and refuses without (D-358, D-389).

D-369 makes rejection teardown independent of the endpoint's claimed stop
state. The function-bound command owner first proves BME clear and excludes it
from the rejected lease's restoration word. Every installed admin and I/O queue
segment then leaves the domain with a drained invalidation before its frame can return
to the allocator; uncertainty retains the segment, controller, device and live
domain together. Only after safe command restoration does the unit revoke the
selected root entry, invalidate and drain it; TE and the empty default-deny root
remain live. Fault diagnostics on this path drain one inventory-bounded batch
after retirement, so a device cannot keep boot inside live MMIO logging.

### Fail closed

Isolation is a precondition of storage rather than a feature of it. If the
firmware reports no DMAR table, or no remapping unit governs the controller's
function, or the unit cannot be taken over, the storage stack does not start and
the boot says `storage: no DMA isolation`. The governing unit is found by
walking each complete DMAR device scope's path through one sealed bridge
topology: an
endpoint scope names one function, a bridge scope names its bridge and every bus
proved below it, the most specific scope wins, and an `INCLUDE_PCI_ALL` unit
takes only what no scope names. The snapshot independently probes every
function number and admits DMA authority only after proving unique parents,
bus-zero roots, contained child ranges and ancestor-related overlaps; malformed
or odd scope paths and an unsealed topology refuse isolation (D-371). Bus
mastering is gated on both active DMA and interrupt boundaries for that exact
function: `admit_bus_master` consumes a `DmaDomain` and an
`InterruptIsolation`, so there is no path that reaches a doorbell without both
fences in place. A degraded boot without the IOMMU device exists to
prove the refusal fires.

D-372 closes the interrupt side of the same admission. The unique DMAR must
advertise interrupt remapping without x2APIC opt-out, the one supported live
DRHD must be segment zero with no RMRR and static whole-segment authority from
either `INCLUDE_PCI_ALL` or a unique matching `WZDM`, every present requester in the sealed
topology must still resolve to it, and the unit must advertise IR, queued
invalidation and extended interrupt mode. Takeover refuses firmware TE, QIE,
IRE or compatibility-format state, installs a kernel-owned two-entry IR table
whose entries remain not present, sets EIME, globally invalidates the
interrupt-entry cache through a private queued-invalidation ring and completion
write, then enables IRE with CFI clear. Before that
transition it masks and reads back FECTL while preserving its reserved bits,
because VT-d's own fault-event MSI never traverses the remapping table. The
exact IRTA, status, FECTL mask and zero entries are read back before the linear
proof can exist.

The installed unit and every requester fence remain owned while maskable
interrupts are on. The empty unit root is installed before PCI topology
discovery, and candidate retirement revokes its context without clearing TE or
IRE or dropping the register mapping and empty IR table; a fallback candidate
retargets only the DMA tables through the same owner. `sti` refuses an unsealed
topology, multiple or foreign-segment remapping units, a PCI machine without
that owner, or a pending vector in any of the local APIC's eight IRR banks.
Because the initial table deliberately admits no source, every I/O APIC line
remains masked too; the keyboard is drained by a poller, and a clockless or
interrupt-disabled poller yields rather than sleeping for a timer that cannot
wake it.

D-439 retains decoded event order across competing polling callers by keeping
each bounded drain-and-delivery batch under one interrupt exclusion, with
sequential locks and reader wakeup after restoration. The self-test arrival
watch observes only events admitted to the queue; it does not reserve them.

The IRR audit is defense in depth, not an architected fabric drain. Firmware,
SMM, devices and remapping-hardware event sources are trusted to remain
quiescent for compatibility-format interrupt traffic until the first successful
kernel IRE establishment. After that boundary devices are hostile and fallback
never disables IRE. Firmware/SMM is separately trusted not to reprogram VT-d or
APIC state after the kernel's readbacks.

If firmware handed the machine over with translation, queued invalidation,
interrupt remapping or compatibility delivery already enabled, the kernel
refuses takeover before its first unit write. A live SRTP to a non-identical
root is not a safe way to inherit that state; VMware qualification therefore
requires an inactive firmware handoff or a future authenticated pre-kernel
owner-transfer protocol.

The tables the IOMMU walks (the root table, the per-function context table, and
the second-level page tables) are ordinary kernel-owned frames written through
the existing memory abstractions, and their physical addresses are programmed
into the unit's registers. Before any unit write, all five table allocations
move into owners whose Drop permanently retains their frames, including the
deny-all interrupt table and private invalidation queue. Read-only preflight
failures still reclaim unpublished allocations. They are never themselves
mapped into any device's second-level tables, so the device cannot reach the
structures that constrain it.

Each DMA segment carries one private mapping-owner identity, derived from the
never-recycled root allocation. Its host-tested frame-number-plus-one encoding
preserves physical frame zero and every aligned address while keeping the
optional owner to one word; unaligned inputs are refused. Mapping records that
identity before publishing
the first leaf and refuses any second mapping. Drop withholds a still-mapped
allocation. Clean refusal before any leaf is published releases the tag without
a drain. After possible publication, only proved rollback or exact-owner
revocation with every leaf cleared and the required DMA drain releases it;
uncertain outcomes permanently retain the allocation (D-433). No heap allocation is
added to the candidate window. Both PCI admission proofs borrow the live VT-d
unit, preventing owner drop or mutation before their last use.
Every live table entry is published by one checked, naturally aligned volatile
qword store. Permission bits occupy the first little-endian byte, so publishing
that byte before the address would transiently authorize a page walk to an
incomplete physical frame. The NVMe mapping boundary also carries direction:
the device may read submission queues and write completion queues and
read-command payloads. Firmware-reserved mappings are refused at lookup rather
than receiving either permission (D-388).
Every device-written payload is cleared before publication, including the first
sector buffer used during controller selection.

### What is deferred, and why it is safe to defer

AMD's IOMMU (PW-711) is a different encoding for the same DMA and interrupt
boundaries and triggers on a qualified AMD host; the Intel unit is what the
qualified hosts present. Non-identity IOVA allocation (PW-712) remains a
separate confidentiality hardening trigger. Each deferral is a named trigger
row with the surface its arrival would open.

### How it is checked

The table parse and the register encoding are host arithmetic and are checked
under `cargo test`: `libs/acpi-tables` parses the DMAR structures, refuses
truncation, odd, empty or out-of-range PCI paths, a region whose base exceeds
its limit, and a duplicate unit, and resolves scopes only through proved bridge
ancestry; `libs/pci-config` seals complete fake topologies and refuses duplicate
parents, unrooted functions, escaping children and unrelated overlaps;
`libs/vtd` encodes the root, context, second-level and interrupt-remapping
entries, the register and queued-invalidation commands, and a takeover predicate
that refuses every active firmware remapping facility; its capability,
coverage and width arithmetic is walked exhaustively. The programming and the fault behaviour need a real unit and are
checked by booting: translation is active for the controller's function and the
unit walks the kernel's root table, storage and the filesystem read through it,
the second-level tables read back reach exactly the live queue frames, a domain
proof is refused while translation is off, and a device write into a frame the
domain does not map faults, changes nothing and shows that the unmap before it
completed a read-and-write drain as well as invalidating the unit's cache. The live checks also require deny-all interrupt
remapping for every PCI requester, exact masked I/O APIC rollback, zero legacy
keyboard interrupts and progress through the poller. The QEMU fixed profile
presents `-device intel-iommu,intremap=on,eim=on,caching-mode=on` and injects
the checksummed `WZDM` authority that exactly names that unit; the degraded gate boots the shipped image without it and
requires the named refusal, and the VMware template enables its virtual unit.
Each check is broken once and its mutation kept.

D-364 separates supported table depth from effective coverage: SAGAW chooses
the walk and every mapped byte stays below both AGAW and MGAW. A supported
deeper table may serve narrower addresses. Two 2026-10-07 VMware boots on
`96c906d` refused width selection and each failed 61 of 271 checks; they are
diagnostic evidence. v0.2.0 closed PW-578 and PW-571 on `694061f` (D-449).
D-365 repairs CRLF and runtime VMX validation; v0.2.0 also closed PW-580.
