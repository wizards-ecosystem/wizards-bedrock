# 014 — Storage (Phase 6)

The phase that first gives the kernel something to keep in flight.

Phase 5 built a ring and gave it Echo. Phase 6 then separated four milestones
that the opening draft conflated: Timer is the first operation whose honest
answer is *not yet*; Duplicate is the first completion that carries derived
authority; OpenAt is the first namespace operation that produces authority;
and Read is the first storage/data caller that needs a handle, offset, buffer
and length together. D-200 later moved Read service to one bounded file worker,
so `RingEnter` now admits the operation and `RingWait` leaves through its saved
trap frame until the worker posts the terminal completion.

Those milestones reshape the ABI, which is why D-149 puts them **before** the
filesystem rather than after it. Build `read()` synchronous as a separate call
and add the ring form later and this kernel has two doors to one operation for
ever — the mistake Phase 5 avoided by freezing the handle table and the queue
before writing their callers.

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
| Nothing blocks on a completion, so the wait queue is built and unused | `011` close | **Slice 1**: Timer becomes the first genuinely deferred operation |
| Port claims prove exclusivity, not device *entitlement* — the device half of the authority debt | D-77; `011` | **Slice 4**, with PCI discovery: a driver claims a device, not a port range |
| The five file sysdeps were OWED or ABSENT under D-114's former taxonomy | D-114; amended by D-181 | Slices 6–9 build the VFS, `OpenAt` and `Read`; D-181 separates the remaining compatibility, door and missing-substrate work |
| `sys_clock_get` was OWED under that former taxonomy | D-114; amended by D-181 | **Slice 2**: monotonic time stops being a call and becomes a load |
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

The order is forced twice over. The ABI slices come first because Timer needs
real deferral and Read is the first storage/data caller to freeze the widened
entry it uses (D-149).
Within the storage half: nothing can read a format without a block device,
nothing can enumerate without a reader, and the VFS needs at least two backends
before its abstraction is anything but a guess.

| Slice | Contents | Status |
|---|---|---|
| 1 | **A completion carries a handle.** Result-slot reservation, same-object attenuation, and the first operation that genuinely waits; cross-type derivation arrives with OpenAt in slice 9 | Done |
| 2 | **The time page.** Immutable constants, a versioned clock with a revocation gate, and proof that kernel Uptime survives revocation; D-181 records the still-incomplete shared userspace fallback | Done |
| 3 | **Death mid-operation.** In-flight operations collected with the zombie; exit is a *wait*, not a signal | Done |
| 4 | **PCI, and a device that is claimed.** CF8/CFC enumeration recursing through bridges, a `Device` capability that supersedes bare port claims, BAR mapping uncacheable | Done |
| 5 | **NVMe, polled.** Twelve-step init, one I/O queue pair, `IEN=0`, PRP construction in `libs/`, and the `size − 1` capacity rule proved | Done |
| 6 | **FAT32, read-only, short names.** `libs/fat32` over a block trait, checked against the ESP `xtask` already builds | Done |
| 7 | **The VFS.** A directory abstraction over resolve, read and enumerate (`modify` is reserved for a future write path), enum dispatch, and path resolution that cannot ascend | Done |
| 8 | **tmpfs and devfs**, so the abstraction has more than one backend and devfs proves that not everything is bytes | Done |
| 9 | **The file handle.** File `READ`, directory `TRAVERSE`/`ENUMERATE`, offsets per operation, and a `Read` submission later deferred by D-200 | Done |
| 10 | **Phase close.** The VMware boot (D-18, and the first one that runs the checks — D-135), the adversarial audit, the roadmap and this file's closing section | Code complete; the post-audit image still awaits the human VMware close |

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
- **Same-object rights attenuate on arrival**, the way D-110 makes channel
  transfer attenuate. A cross-type producer may derive only a type-specific
  right authorised by the source operation and preserves no universal right
  the source lacked; OpenAt's `TRAVERSE`-to-`READ` rule is the worked case.
- **Exactly one terminal completion still holds.** Cancellation is itself an
  operation with its own reservation and its own completion, and never
  substitutes for the target's — `012` §8, and the antidote to the double-completion
  bug class that produced two io_uring CVEs.

### The time page is global and has two regions (D-152)

An **immutable constants region** — tick rate, page size, ABI version — with no
seqlock and no version counter, because there is nothing to tear. Zircon's
model, and it removes the most error-prone reads from the protocol entirely.

A **versioned clock region** — Linux's seqlock protocol with both barriers,
whole `base_ns` plus a fractional carry, a 128-bit multiply/shift in the
reader, and a `clock_mode` **revocation gate**. A kernel check proves ring
Uptime still answers after revocation. The shared userspace client that can
correlate its fallback completion remains Phase 7 work (D-181).

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

**The formerly budgeted one-writer weakness is closed.** D-207 pins a volume
formatted and populated by `dosfstools`/`mtools`, with adversarial valid layouts
which both host readers must interpret identically. The local `fatfs` corpus
remains the one the kernel boots.

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

**The abstraction is over directories**, because path resolution is the code
that must be backend-agnostic. The delivered surface resolves, reads and
enumerates through enum dispatch; mutation waits for a write path. It is not a
trait pretending a backend can be added without editing the dispatcher.

**Enum dispatch while all three backends are in-kernel** — monomorphic,
allocation-free, exhaustively checked when a fourth arrives. Promote to dynamic
dispatch only when a backend must be added without editing the enum, which here
means *when a filesystem lives in userspace*, and at that point the abstraction
is the protocol and the trait is irrelevant.

**devfs does not go through a byte interface.** A devfs entry has no bytes; it
resolves internally as `Node::Device`. Forcing it to implement `read_at` is the
"everything is a file" conflation, and WASI's own README rejects it on exactly
these grounds. OpenAt currently refuses that node; a user-visible `Device`
handle remains Phase 7 work.

### Rights, and the one split worth stealing (`012` §4)

The four universal rights are already, bit for bit, Zircon's basic rights. Per
type:

- **File:** `READ`. `WRITE` remains meaningful for Channel but no current File
  admits it. Raw `EXECUTE` and `MAP` are defined by `handle-space`, but no
  current object admits either.
- **Directory:** `TRAVERSE`, `ENUMERATE`. Raw `MODIFY` is reserved but no
  current Directory admits it while the namespace has no mutating operation.

`TRAVERSE` versus `ENUMERATE` is the valuable one: "may resolve a name I know"
versus "may list the names", and a real kernel distinction by D-78's rule
because the resolver consults one and readdir the other. Capsicum folds
enumeration into read; WASI Preview 1 had both by accident of being
syscall-named.

**Rename and link split authority by side.** They are the only operations
taking two directory handles, and Capsicum's man page notes that
`CAP_LINKAT_SOURCE` "allows linking files with broader rights" — folding the
two into one bit is an **authority-laundering vector**.

**One set, with same-type authority monotone non-increasing.** OpenAt does not
accept a requested mask. A directory preserves its parent's rights unchanged;
a file preserves only universal rights already held and derives `READ` from
`TRAVERSE`. That cross-type derivation is not a bitset subset, but it cannot
recreate a universal right or grant an unauthorised file operation. WASI
Preview 1's base/inheriting pair forced its libc to compute two masks per open
and was deleted.

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

### How few threads eventual deferred reads need (`012` §8)

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
- **A same-object Duplicate with rights it was not offered**, which must be
  refused, plus an OpenAt derivation that must not restore universal rights.
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
- **POSIX `sys_open` remains partial platform and compatibility work.** Native
  OpenAt resolves existing paths from a directory handle, but ring 3 cannot yet
  receive the first one. The adapter then supplies root/cwd/descriptor state
  and returns EROFS for creation, truncation and write modes rather than
  inventing a write path (C-3, D-181).

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

**Landed 2026-08-25.** D-163, D-164, D-165. 151 machine checks, 610 host tests,
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

- **At this slice, `Pending` was a timer and nothing else.** D-41 forbade the
  abstraction with one implementation. D-200 later added queued and running
  Read kinds when deferred device service arrived.
- **No worker thread.** `012` §8 is emphatic, and a state machine driven from
  the waiting thread needs none. Sixteen thread slots is the reason and the
  measurement will be taken in slice 5.
- **`Queue`'s `WaitQueue` is still unused by ring 3**, and that is not an
  oversight: it takes a `ThreadContext`, so it is what a *kernel* thread waits
  on. A ring-3 thread waits through the scheduler.

---

## Slice 2 — The time page

**Landed 2026-08-25.** D-166. 156 machine checks, 619 host tests, 37 mutations.

Under D-114's former taxonomy, `sys_clock_get` was OWED over
`libs/timebase`—the mechanism complete and the door missing. Slice 2 answered
the monotonic path by not making it a door. One page is read-only in every
program at `0x0000_1000_0000_0000`, with constants in one region and the clock
in another; D-181 separately records the C reader and fallback as partial.

### What the research asked for, and what turned out to be wrong

**"nsec stored pre-shifted"** cannot be taken literally. `base_ns << shift`
overflows a `u64` after about eight seconds of uptime at a 1 GHz counter's
shift, so the base is split — whole nanoseconds beside a carried fraction of
one, which is what Linux does and for the same reason. The fraction is not a
detail: a clock updated at a kilohertz that dropped it falls a microsecond
behind every second.

**This clock makes no periodic adjustment, and that limit is explicit.**
`time::init` normally accepts two believable PIT windows; a complete CPUID
0x15 ratio on an invariant TSC can supply the rate when those windows refuse,
while leaf 0x16 stays diagnostic. D-225 retains the claimed PIT to validate
every bounded APIC appointment. Two consecutive conclusive APIC/TSC
disagreements freeze public time, revoke the page and unwind every Timer; late
or folded windows are inconclusive. PIT samples continue a private scheduler
epoch so an already-running program remains preemptible without publishing
untrusted elapsed time. Suspend and NTP adjustment remain unsupported.

### Three things that were wrong first

**The page has to exist even when the clock does not.** The ABI says there is a
page at that address, and a program that reads it and faults has been lied to.
It is built at boot whatever the clock did, `mode` is `MODE_NONE`, and a spawn
that cannot show it is refused.

**The preemption precondition was measured in the wrong place.** The first
counter recorded *calls* made with interrupts on, and the check built on it
failed immediately — correctly, because a kernel thread calling `publish` with
interrupts on is fine. What matters is the window, and the guard closes it.

**The odd marker has no witness on one CPU.** Deleting the write that opens the
window left every check green: nothing else runs between the two increments.
This is D-154's class, and the answer is the same — the writer reads its own
marker back, so a writer that stops announcing itself is caught by the only
observer that exists.

### The check that has no tolerance

*The time page and the kernel clock tell the same time* is a **sandwich**: the page's
reading is taken between two `now_ns` reads and must lie between them. Not a
bound, not a tolerance — a multiplier or shift off by one fails at once, and
there is no slack for it to hide in.

### What moved in the ABI

`WIZ_CALL_CLOCK_GET` is **deleted**, and its absence is the point.
`WIZ_CALL_RING_WAIT` takes 3. At this slice the speculative owed calls
renumbered, which cost nothing while none was implemented; D-181 later deleted
those unimplemented numbers. The C header gains the page's address, version,
modes and every field offset, and the cross-check compares all of them against
`timepage.rs` — a `mode` read from the wrong place is a clock that reads as
usable when it is not.

---

## Slice 3 — Death mid-operation

**Landed 2026-08-25.** D-167. 158 machine checks, 619 host tests, 39 mutations.

Until slice 1 there was nothing to say here: every operation completed inside
the call that submitted it, so a retiring program had no work in flight by
construction. `Opcode::Timer` made D-153's debt reachable in the same commit
that created it.

**The hazard is specific.** A retired program's ring pages go back to the
allocator in `reclaim`. An operation in flight is one that will later write a
completion, and the page it would write into is about to be somebody else's.
Not a leak — a write to freed memory, arriving whenever the operation finishes.

**So the ring is quietened first.** `abandon` ends every operation and posts the
completion it was owed, into pages the zombie still holds. The completions go
into a ring nobody will read, and that is not waste: posting them is what
spends the reservations, and a reservation never spent is a ring whose
accounting says work is outstanding for ever.

**No grace period, no refcount** — `012` §10's third consequence, and the
easiest to violate by accident. A teardown that waits for a count to reach zero
*eventually* is precisely the out-of-line cancellation that took io_uring six
years to undo.

### The part that needed no code

D-151's third terminal path is "a retired program's", and D-163 had already
made the reservation a value holding an `Arc` to its table. A program that dies
holding a promise leaves the table alive exactly as long as the promise, and
the destructor gives the slot back and lets the table go. The teardown drops
its reference and the table simply does not leave.

That is *checked* rather than asserted: the check watches the table through a
`Weak`, spends the promise against it after its owner is gone — surviving and
working being different claims — and then requires the `Weak` to stop
upgrading, because a table nothing will ever free is the other failure.

### What slice 5 inherits

The shape, not the code. A polled NVMe read cannot be ended by writing a
completion: the device has the command. `abandon` will have to wait for the
controller there. What it must not do is return before `is_quiet`.

---

## Slice 4 — PCI, and a device that is claimed

**Landed 2026-08-25.** D-168. 161 machine checks, 637 host tests, 45 mutations.

D-77 closed half an authority debt and named this half: a port claim proves
*exclusivity* and says nothing about *entitlement*. A port range has no
identity and cannot be asked what it is.

**A driver now claims a function, and the claim carries its windows.** That
settles the first of this file's open decisions — a device claim *contains* the
port claim rather than authorising one — because two mechanisms with a gap
between them is how a driver comes to hold a device capability and ports that
have nothing to do with it.

### What the machine decided rather than taste

**CF8/CFC.** ECAM needs an MCFG table; QEMU's i440fx publishes none and VMware
emulates a 440BX. The 1992 mechanism is the only one on both targets D-18 makes
this project answer to.

**The check machine had to be given a bridge.** q35 as QEMU ships it has none,
so a kernel that scanned bus zero and stopped would find all six functions,
report a plausible number, and pass — while being unable to find a controller
on VMware, which puts functions behind `pciBridge0`–`pciBridge7`. The survey
counts **buses**, not only functions: the function count says nothing about
descent.

**A bridge has two base address registers, not six.** Its bus numbers sit where
a device's third would be, and a claim that read six would *probe* them —
writing all ones into the register that says which bus is behind the bridge.
The probe restores what it found, so nothing downstream need notice; the check
asserts the claim decodes two.

### Two things caught by the harness on their first run

**The window sizing was wrong in the I/O case.** One function taking "pass all
ones for a narrow register" complemented a thirty-two-bit mask in sixty-four
bits and answered eighteen quintillion. A doctest caught it. It is two
functions now, because a width you must remember to pass correctly is a width
that will be passed wrongly.

**`pci::init` took a lock under a leaf guard.** It held this module's guard and
then called `port::claim_pair`, which takes the port registry's. The machine
check went red on the first boot and named both sites. The claim happens
outside the guard now.

### Deliberately not yet

The `Device` is a **kernel-side capability, not a handle**: obtainable only
from the registry, exactly as `Port<T>` is. Slice 8 makes it resolvable
internally through devfs but does not create a handle; the first user-visible
Device handle remains Phase 7 work (D-41, D-172).

---

## Slice 5 — NVMe, polled

**Landed 2026-08-25.** D-169. 165 machine checks, 654 host tests, 48 mutations.

The kernel reads a block off a real controller. On QEMU that is
`1b36:0010` at `00:04.0`, claimed by slice 4's registry, its BAR0 mapped
uncacheable, brought up through the specification's sequence, given one I/O
queue pair with `IEN = 0`, and asked for logical block zero.

```
nvme: claimed 00:04.0 — 1b36:0010
nvme: memory space and bus master on
nvme: bar0 is 16384 bytes of 64-bit memory at 0xc020000000
nvme: up — 8 entries a queue, 4096 blocks of 512 bytes, doorbells 4 apart
nvme: read block 0
```

### The ring parallel is a trap, and the arithmetic is shared without it

Four things differ from `kernel::queue`, and each is a bug if assumed away:
there is no completion tail to read, so entries are found by their **phase
tag**; completions are matched by **command identifier**, not by slot;
**capacity is `size - 1`**, whose symptom is a hang; and the doorbell is
**write-only**, so the tail is shadowed. `libs/nvme` holds the arithmetic and
there is no shared `Ring<T>`.

### Two checks that could not have failed

**A disk of zeros proves nothing.** Reading block zero out of a zero-filled
file and comparing it to zero is what a buffer nobody wrote also contains. So
`xtask` writes a different signature into every block, and the check reads four
of them — a driver that ignored the logical block address would answer block
zero to everything and pass against a uniform disk.

**Reading one block too many is invisible at the front.** `CDW12` carries
`blocks - 1`, and a driver that sent the count unadjusted reads two blocks with
the first block's bytes still exactly right. The check went green against that
mutation. It now clears the buffer and requires the bytes **after** one block to
still be zero: the extent of a read is only observable past its end.

### One break that was recorded red and was not

*A register file the cache may answer for* was reported red when this slice
landed, and it was a **build failure**: the break wrote `Flags::empty()`, which
does not exist, and a boot that does not build produces no pass line — which the
script read as a failing check. The phase close's mutation sweep found it
(D-175). The property is verified now, measured at the page-table entry: the
device window is `0x…0073` normally and `0x…0063` with the mutation, and bit 4
is exactly `PCD`.

### What is left for later slices

A read still spins, bounded rather than endless. Slice 9 gave it a submission
shape; D-200 later moved the polling to a preemptible worker and coupled its
queued/running state to slice 3's teardown rule. An interrupt-driven controller
remains future work.

---

## Slice 6 — FAT32, read-only, short names

**Landed 2026-08-25.** D-170. 169 machine checks, 670 host tests, 52 mutations.

The kernel mounts a FAT32 volume off the controller and reads files out of it.

```
fat32: mounted — 129046 clusters of 512 bytes, 2 file(s) and 1 directory(ies) in the root
```

### Two of this file's open decisions, settled

**The block layer is a trait, and it belongs to the reader.** `libs/fat32`
defines `Sectors` because that is the parameter it needs — not a speculative
kernel-side abstraction over a device this kernel has one of. The host backs it
with a byte slice, the kernel with the controller, and the *same reader* runs
over both. That is what makes the oracle possible at all.

**No page cache.** The reasoning `014` gave holds: whether reading the same
sectors repeatedly is affordable is a measurement, and the measurement does not
exist. The volume is mounted per call for the same reason.

### The oracle

`xtask` builds one corpus with `fatfs` and it goes two places: onto the NVMe
namespace at sector 2048, and into a host test that reads it with **both**
`fatfs` and `libs/fat32`. Two independent implementations, one corpus, and the
disagreement is the failure — which beats depending on the mature one, because
a dependency cannot disagree with itself.

### Four machine breaks passed at first

The end-to-end checks could not reach the twenty-eight-bit entry mask (the
corpus's reserved bits are zero), the file-size bound (the check's buffer was
cut to the file's length), the deleted marker (no deletions in the corpus) or
the split first-cluster field (every cluster fits sixteen bits). All four are
caught by host tests over synthetic entries, which is the right division — but
two were reachable on the machine for almost nothing:

- the corpus now creates a file and **deletes it**, leaving an `0xE5` slot
  between live entries, and the check counts the survivors;
- the read check offers **a whole sector of buffer for a file that is not one**,
  so a reader bounded by the cluster hands back its previous occupant's data.

D-165's rule again, arriving through the corpus rather than through the set-up.

### Carried deliberately

512-byte sectors only, refused rather than misread — that is what lets every
buffer be a fixed array. D-207 adds the independent writer evidence: a pinned
foreign volume forces both readers through a multi-cluster directory,
deleted/long-name slots and a fragmented file without changing the boot corpus.

---

## Slice 7 — The VFS

**Landed 2026-08-25.** D-171. 173 machine checks, 680 host tests, 56 mutations.

A compact surface rather than five tables: resolve, read and enumerate today,
with mutation reserved until there is a write path. Path resolution is the code
that must be backend-agnostic, so it is what the abstraction is about.

**Enum dispatch**, monomorphic and allocation-free, exhaustively checked by the
compiler when a fourth backend arrives. Dynamic dispatch waits for a backend
that must be added *without editing the enum* — which means a filesystem in
userspace, and at that point the abstraction is the protocol.

### The resolver has no way up

`libs/path-walk` resolves `..` lexically and refuses underflow, so what reaches
the resolver is a list of names that only descends. There is **no parent
pointer anywhere in `vfs.rs`** — not a private one, not an unused one. That is
stronger than `RESOLVE_BENEATH`, which enforces a predicate on a resolver still
*capable* of ascending; here there is no capability for a predicate to be about.

Two refusals, and they are different things. An absolute path is malformed
because **there is no root** for it to be relative to; an escaping `..` is
malformed because there is no parent. Neither is a denial — reporting either as
a permission failure would suggest some other program could.

And nothing is clamped: an escape is refused rather than resolved to the
starting directory, a long name refused rather than truncated, a deep path
refused rather than cut short. Each would silently answer a different question
than the caller asked, and on a walk that ends in an open, open a different file.

### What hung the harness

The enumeration check's first version resolved each listed name from *inside*
the enumeration callback. Both operations reach the controller through a leaf
guard, so the inner call re-entered the same lock and spun — reported as a
sixty-second timeout, which is `lock_leaf`'s rule arriving in the only form a
same-lock re-entry can take. The "no lock under a leaf guard" check watches for
a *different* lock; a spin on the same one is not something a counter can see.

The check collects the names and resolves them afterwards, which is also the
shape a caller must use, and the comment says so.

### Carried

Offsets are per operation — no seek, no tell, no cursor on anything. One
namespace per program and no way to compose two. Enumeration and traversal are
separate authorities and the check requires them to agree; the rights that gate
them arrive in slice 9.

---

## Slice 8 — tmpfs and devfs

**Landed 2026-08-25.** D-172. 176 machine checks, 680 host tests, 59 mutations.

```
fat32: 2 file(s) and 1 directory(ies) in the root
tmpfs: 1 file(s) and 1 directory(ies) at the root
devfs: storage is a block device
devfs: console is a character device
```

Two more backends, chosen to be unlike the first in different directions.

**tmpfs** has no clusters, no chains and no device; its directories are a flat
table with parent indices. If the three operations fit both it and FAT32, they
are about directories rather than about FAT — which is the whole claim this
slice exists to make.

**devfs** is unlike both in the way that matters most. `Node` gains a third
variant and `vfs::read` takes a `File`, so **there is no way to pass a device to
it** — not a refusal at run time, an absence at compile time. That is the
difference between a design that says devices are not files and one that
enforces it.

### What the enum did

Adding a `Node` variant was a compile error in every match that handles one,
which is exactly what slice 7's enum-dispatch decision was for: a fourth backend
is a build failure rather than a number that means nothing.

### The corpus caught a check again

A tmpfs lookup that **ignored the parent** passed, because every name in the
first tree was unique — a lookup that searched the whole arena found the right
entry anyway, and deleting the containment check left the boot green. The tree
now holds `MOTD` twice, once at the root and once inside `NOTES` with different
content, and the check reads both.

D-155's rule for the third time this phase, and the third different place it has
arrived from: an input, a corpus, and now a directory tree.

### Checked across backends, not within one

The no-ascent invariant is exercised on **every** backend, because it is a
property of the resolver — a check that only ran against FAT would be a property
of FAT. And a directory handle names its own backend: each name must resolve
where it belongs *and* nowhere else, so the refusals are about the backend
rather than about the name.

---

## Slice 9 — The file handle

**Landed 2026-08-25.** D-173. 179 machine checks, 680 host tests, 62 mutations.

A ring operation resolves a path from a directory handle, gets back a file
handle carrying derived, non-escalating authority, and reads bytes off the
controller into a shared page. That is the whole phase in one sentence, and
every part of it was built by a different slice.

### The last three of this file's open decisions

**A `File` handle's object is the thinner one** — a backend and a node within
it, not an inode the VFS owns. The fatter one is what a write path and a page
cache both want, and this phase has neither.

**`read` takes a provided buffer, and the buffer is part of the ring.** A third
shared page is the data area, so it is always mapped, always the right size, and
its bounds are two comparisons. There is no user pointer to validate, no SMAP
window to open and no fault to recover. The cost is one copy, named rather than
hidden: zero-copy needs memory objects, which is a later phase.

**`OpenAt`'s rights are derived, never asked for.** A directory preserves its
parent's same-type rights. A file preserves only universal rights already held
and derives `READ` from `TRAVERSE`; it does not recreate a universal right.
`WRITE` is never granted, because there is no write path and a right that grants
nothing is a syscall name in disguise.

### The entry widened, and this was the moment

D-149 put the ABI slices first because a caller freezes the interface it uses.
Read is the first storage/data caller. Sixteen bytes could not hold a handle,
an offset, a buffer and a length; the entry is thirty-two now, which is cheap
today and an ABI break tomorrow. Timer, not Read, is the first deferred caller.

At slice close four transcriptions existed — kernel parser/serializer, C
header, emitted ring-3 program and test helpers — while the host check compared
only sizes and stride. D-181 repaired the gap: one kernel offset map now feeds
parser, serializer and helpers; the check derives C offsets, widths and sizes,
then checks actual emitted-ELF operands plus entry base and count. A persistent
`argument`-offset mutation must make the program-ring check red.

### The check that changed no output

Three checks could not fail, and the third is the one worth recording.
Removing the buffer bound from the submission path changes **nothing
observable**: the shared page bounds the copy anyway, so the completion carries
the same status either way. What it changes is *when* — a bound checked after
the read is `-ENOBUFS` with the work already done.

Making that visible took a counter on the driver. The controller's command
identifier advances once per command, so "the device was not asked" is a
number: the check submits a readable handle with a buffer running off the end
of the data area and requires the count unchanged.

### The slice's named limit

**A ring-3 program still cannot reach any of this.** `spawn` gives a program
exactly one handle — its bootstrap channel — and there is no ring-3
channel-receive operation yet. Giving a program a second handle at spawn would
break the invariant that makes pillar 1 checkable, so it waits for the channel
rather than for a convenience.

---

## Slice 10 — Phase close

**2026-08-25.** D-174–D-179.

### What the phase is

A ring operation resolves a path from a directory handle, gets back a file
handle carrying derived, non-escalating authority, and reads bytes through the
NVMe device interface into a page it shares with the caller. QEMU and VMware
exercise virtual controllers; real hardware remains Phase 12. Every part of that
sentence was built by a different slice, and the order was forced: the ABI
first, because a caller freezes the interface it uses.

| | |
|---|---|
| Machine checks | 213 |
| Mutations | 261 registered; 64 in the last recorded full sweep, with 197 later entries broken individually |
| New `libs/` crates | `nvme`, `pci-config`, `fat32`, `path-walk` |
| Decisions | Phase decisions D-163 … D-179; post-phase corrections D-181, D-182 and D-185 … D-228 |

The two counts are **checked against the source at every commit**
(`xtask/src/prose.rs`), but that proves registration rather than a current full
sweep. The code registry carries the entries; this table's count is its checked
restatement. A number copied anywhere else can disagree with both — which the
per-slice lines above did, in seven places, and which is why they now say what
the gate said rather than what was expected of it (D-175). The host-test count
is deliberately absent: it changes with every
`libs/` crate and nothing would be learned from pinning it here.

### Every debt this file opened with

| Debt | Paid |
|---|---|
| No completion carries a handle | Slice 1 (D-163) |
| Nothing blocks on a completion | Slice 1 (D-164) |
| Port claims prove exclusivity, not entitlement | Slice 4 (D-168) |
| `sys_clock_get` OWED under D-114's former taxonomy | Slice 2 — monotonic time stopped being a call (D-166); D-181 records the reader and fallback as PARTIAL |
| An in-flight operation whose program retires | Slice 3 (D-167) |
| `xtask/src/elf.rs` transcribes ELF offsets | Program loading paid early in D-158; D-213 moved the remaining section-table file-header fields into `libs/elf64` |
| A wait with a timeout | Slice 1 (D-164) |
| KASLR ledger entries | Slice 2's time page; slice 4 needed none — the NVMe window is discovered, not fixed |

The five file sysdeps remain incomplete for distinguishable reasons.
`sys_read` has a native explicit-offset Read, but needs a reachable file
handle and the compatibility layer's shared cursor. `sys_write` has no write
path. `sys_seek` changes that same POSIX cursor, which the native ABI
deliberately omits. `sys_close` has an internal kernel mechanism, but releasing
the last object reference can deallocate; it still needs a deferred or otherwise
trap-safe user route. `sys_open` layers root/cwd/descriptor policy and explicit
EROFS refusals over read-only OpenAt, but also lacks the ring-3 route that
delivers its first directory. D-181's exact register, narrowed by D-182, keeps
those routes separate.

### The five decisions this file left open

1. **A device claim contains the port claim** rather than authorising one
   (D-168). Two mechanisms with a gap between them is how a driver comes to
   hold a device capability and ports that have nothing to do with it.
2. **The block layer is a trait, and it belongs to the reader** (D-170).
   `libs/fat32` defines it because that is the parameter it needs — and it is
   what makes the host oracle possible.
3. **No page cache** (D-170). The reasoning held: the measurement does not
   exist, so neither does the cache.
4. **A `File` handle's object is the thinner one** (D-173) — a backend and a
   node, not an inode the VFS owns.
5. **`read` takes a provided buffer, and the buffer is part of the ring**
   (D-173). No user pointer to validate, no SMAP window, no fault to recover.

### What the audit found

One defect in three places, and a claim nothing checked (D-174). Three words a
submission carries — a rights word, a file offset, a cancellation token — were
reaching the kernel through `as` casts, so a program was answered about
something other than what it asked for. And `devfs::DEVICES` documented a test
that did not exist.

The rule for next time: **every field a submission carries is a value ring 3
chose.** `as` is not a conversion at that boundary.

### The VMware boot, which is not yet done

D-18 makes a VMware boot the definition of done, and D-135 said this would be
the first phase close whose VMware boot runs the checks rather than only
reaching a prompt. It ran them, and it came back `verdict v1 FAIL passed=117
failed=63`.

Both causes are recorded in D-176 and both are fixed in the tree; neither was a
defect in what the phase built.

**The VM had no NVMe controller.** The VMX template has declared a SATA boot
disk since Phase 2, and nothing added the controller `012` §2 chose the driver
for. Ten slices were developed against a machine that could satisfy them and
shipped to one that could not. `cargo xtask vmware` now builds a second VMDK
over the same namespace image QEMU is given, and a host test reads the template
for the controller so this cannot revert quietly.

**The clock refused to calibrate and did not say which of six guards refused.**
About forty checks stand on a running clock, which is most of the remaining
failures. The warning now names the guard and prints the two rates the window
measured, believed or not.

**Then four more, and they had never run.** With the controller in place the
same machine reported `passed=172 failed=9`, and four of the nine named
physical `0x1000` — a page QEMU describes as memory and VMware does not. Each
returned `false` out of its own setup, which is the value the harness also gets
when a property is violated, so four checks that never executed were reported
as four security properties coming apart. They cut their pages from a run the
map offers now (D-177). All four pass on VMware, where none of them had ever
been evaluated.

**And one about the keyboard, still open.** `passed=177 failed=5`. Four of the
five need a human at the machine to press a key and cannot run headless. The
fifth is real, and the boot that added D-178's identity probe answered it:

```
[info ] selftest: the controller raises its line exactly when routed, and never translates
[warn ] keyboard: reports scancode set Some(65), not 2
[warn ] keyboard: identifies as Some((171, 65)), not (171, 131)
```

`(0xAB, 0x41)` is an MF2 keyboard **behind a translating controller** —
`(0xAB, 0x83)` is the same keyboard when nothing rewrites it, which is what
QEMU answers. So VMware reports its translation bit clear, passes the check
that reads that bit, and translates anyway. The decoder is being fed set 1
while it decodes set 2, which is precisely the symptom that started this: keys
are accepted and nothing happens.

Two checks on the same boot disagreed, and the one that could not be lied to
was right. That is D-178's whole argument, established on the machine rather
than reasoned about.

**Set 1 is decoded now** (D-179). Not by untranslating — set 1 spells Pause in
six bytes where set 2 uses eight, so a byte filter would need the state machine
it exists to avoid — but by a second table and a second machine, sharing the
key identities, the shift state and the layout, which is all the two sets have
in common.

The second transcription pays for itself as a **check on the first**: the two
sets are different numbers for the same hundred-odd keys, so
`both_sets_name_the_same_keys` fails if either drops a key, shadows an arm, or
files one in the wrong half. D-156 resolved a single backslash against two
sources by hand; the other hundred are now covered by construction.

And the self-test reaches the set 1 path without a human, by making QEMU
translate — the gate boots with the controller in the state VMware is in
unasked, and presses a key whose identity the harness chose.

**The 2026-08-25 image stood at `passed=182 failed=5`, and all five needed a
human.** They were the two injection rounds — a keystroke cannot be injected
into a headless VMware run the way QMP injects one into QEMU — and the three
checks that read what the first round delivered. Every other check in that
image was green, including the ten storage checks that had no device, the four
paging checks that had never run, and both keyboard checks once they stopped
claiming a set the kernel no longer requires. D-181 and D-182 post-date that
measurement, so the current 189-check image still needs the same human round;
the old total is evidence about that dated image, not a current verdict.

**So the phase is closed on the code and open on one boot with a person at it.**
The count in the table above is QEMU's. This section records what VMware
actually said rather than a boot that was not got — which is what D-77 forbade,
and what `012` §11's "VMware's values are unmeasured" already admits about the
neighbouring number.

One thing stays unexplained on purpose: the clock refused to calibrate on the
first of these boots and calibrated on the five that followed, with no change
to the calibration path between them. That is why D-176's diagnosis prints the
guard and the two measured rates rather than a verdict — the next refusal will
say which of seven it was, and until one happens there is nothing honest to
write here.

**What the four boots were worth.** Three defects and a rule apiece, none of
them findable on QEMU, in a phase that closed green: a driver shipped to a
machine with no such device, four load-bearing paging checks that had never
executed anywhere but one emulator, and a keyboard whose configuration byte and
whose behaviour disagree. **A second machine does not re-run the checks; it
discovers which of them were about the first machine.**

### Carried deliberately

No write path. No page cache. No symlinks or hard links. One namespace per
program with no way to compose two. POSIX `sys_open` still needs a ring-3 route
for its first delegated directory, compatibility root/cwd/descriptor policy and
explicit EROFS handling for mutating modes. D-207 closes the former one-writer
corpus limitation with pinned independent formatter output while retaining the
local corpus as the exact booted bytes.

### What this phase taught the next one

D-155's rule — choose inputs where the answers differ — arrived from four
directions in ten slices: an input, a corpus of disk blocks, a directory tree,
and a rights word whose low half was valid. D-165's — a check must reach the
code it is about — arrived three times. Both are in `005` now, and both were
found by breaking checks rather than by reading them.
