# 013 — The application platform (Phase 6, slices 1–3)

What a program is allowed to ask for, decided before the first deferred ring
operation was written.

Phase 5 fixed two interfaces this project will live with longer than any other
— the handle table and the queue ABI — and said so at the time: both are the
kind that get frozen by their first caller. The caller Phase 5 gave them echoes
a word back. Phase 6 eventually supplied three distinct pressures: Timer was
the first deferred operation and proved ring wait; Duplicate first proved a
completion could carry authority, with OpenAt as the motivating namespace
caller; Read was the first storage/data caller. The interface they froze is
not the interface the then-two-opcode ring had frozen.

This file exists so that the freezing is a decision rather than a side effect.
It is a charter per D-19, written before its code, and it is deliberately
small: three constraints on Phase 6, three pieces of work, and one reservation
that costs a paragraph now and a migration later.

**It is not a phase of its own, and no longer claims to be.** It was drafted
as "Phase 6.5", to run after the filesystem. That ordering was rejected
(D-149): storage is the first external-I/O subsystem that eventually needs
blocking service, so building it first and extending the ABI afterwards would
leave `read()` with two doors. Timer proved the deferred/wait contract before
the polled storage implementation arrived; the filesystem then became its
first storage consumer. These became **Phase 6's first three slices**.

Nothing here is a subsystem and no boot closes it. What closes it is that the
VFS slices open against an interface nobody has to re-cut.

References: `010-research-notes-3.md` §2 (the async ABI and its four hard
problems), §3 (capabilities and the handle table), §5 (the POSIX bill), §12
(what must be decided before code); `011-userspace.md` throughout, and
`userland/README.md` for the sysdep finding that motivates constraint C-3.
Outside the project: io_uring's man pages and its CVE record; Watson,
Anderson, Laurie and Kennaway, *Capsicum: practical capabilities for UNIX*
(USENIX Security 2010); the WASI 0.3 clocks and filesystem interfaces; Fuchsia
RFC-0028 and the Zircon vDSO documentation; Fuchsia's component manifest
format; Linux's vDSO `clock_gettime`.

D-183 gives this language-neutral work its target consumer. The Rust kernel
provides handles, queues and lifecycle; every feasible first-party program over
them is Wizzy. Until `015-wizzy-userland.md`'s compiler-readiness gate opens,
new platform work must name a concrete future Wizzy caller rather than grow a
parallel Rust userland.

D-184 and `016-pre-wizzy-foundation.md` hold the finite closed-gate work
register. A `TRIGGER` or post-gate row there remains stopped even when this
historical charter describes the eventual platform need.

## What earlier phases left owing

| Debt | Where recorded | What 6.5 does |
|---|---|---|
| No completion carries a handle, so the ring cannot be the delegation path the channels are | `011` close | Pays it, in the phase whose first operation produces one |
| The program's stack is the boot-time arena, so sixteen is a cap on concurrency | D-117; `011` close | **Not here.** Named so nobody mistakes it: it is Phase 7's, and a shell reaches it immediately |
| `xtask/src/elf.rs` transcribes the ELF header rather than using `libs/elf64` | D-105; `011` close | **Not here**, but the note reservation below is the second caller that makes the duplication cost something |
| A program receives exactly one handle at spawn and must negotiate the rest | D-111 | Reserves metadata that lets a spawner decide and enqueue bootstrap grants before the first instruction; does not build delivery |

## The principle

**The ABI must never require one kernel thread per outstanding operation.**

This is the one sentence the rest of the file is downstream of, and it does not
depend on anything above the kernel. Sixteen concurrent programs is today's cap
(D-117), so thread-per-operation is already the wrong shape *for this machine*.
Above that: an event loop, a server holding many connections, or any scheme
that multiplexes work onto fewer threads than it has work all die against a
door that can only block. The ring is already completion-based. The principle
only says not to undermine it when a synchronous door would be quicker to
write.

io_uring is the prior art and also the warning. Its model is right and the
industry moved to it; its *implementation* accumulated enough exploitable bugs
that Google disabled it by default in Android and ChromeOS in 2023. The
difference is opcode sprawl, kernel worker threads, and revalidation of shared
memory — three things the then-two-opcode ring did not have. The current ring
has seven opcodes and still no worker pool; Phase 5 already refused the last by
construction (`queue.rs`: the copy is what is used, and a second read would be
the double fetch the design refuses).

## Three constraints on Phase 6

These are rules, not code. Each is forced by Phase 6's own content, and each is
cheaper to state now than to discover once the VFS has callers.

### C-1 — No blocking-only door

Every operation that can wait has a ring form. Operations that cannot wait may
stay direct. At adoption the examples were the clock, `exit` and `tcb_set`;
D-166 later made the clock a page load, while Exit and TcbSet remain direct.

Storage supplied the first external-device operation that must eventually
defer rather than poll inside `RingEnter`, which is why the rule landed before
its VFS. Timer became the actual first deferred operation and proved the wait
contract. The current polled Read has the right ring shape but has not yet paid
the asynchronous device-service half.

### C-2 — A completion may carry a handle

Recorded as a debt at Phase 5's close; Phase 6 is where it stops being
theoretical. Duplicate was the first implementation and proof; OpenAt is the
namespace operation whose useful result is newly derived authority.

Without it there are two delegation mechanisms with different powers: channels
can move a handle and the ring cannot, so every authority-producing operation
detours through a channel — two round trips and two mechanisms for one idea.
The near prior art is io_uring's *direct descriptors* — `IORING_OP_ACCEPT`
and `IORING_OP_OPENAT` with `sqe->file_index` set, installing into a
ring-private table rather than the process's. (`accept_direct` and
`open_direct` are liburing helpers; there are no such opcodes.) The model for
the *capability* half is Zircon, where a channel message carrying handles is
the ordinary case rather than an extension.

### C-3 — The filesystem has no root

Every name resolves from a directory handle the program was given. No global
root appears, in Phase 6 or later.

This is already the design — pillar 1. Native OpenAt now exists, so D-181
records `sys_open` as PARTIAL rather than absent: ring 3 still cannot receive
the first delegated directory, and a POSIX root and current directory may be
constructed only from directory handles the process was delegated. The constraint is written
here because a VFS is exactly where an ambient root grows out of convenience,
and because the alternative is settled elsewhere: Capsicum reached openat-only
in 2010, and WASI's preopened directories and Fuchsia's per-process namespaces
arrived at the same answer independently.

The cost is known and already measured rather than estimated: a libc built on
this cannot serve `open("/etc/foo")` to a process nobody gave a root to. That
is the point rather than a defect.

## Three pieces of work

### W-1 — Completions carry handles

C-2, implemented. The rights question is the interesting half. Same-object
Duplicate must attenuate the way channel transfer does (D-110). A cross-type
producer may instead derive only a type-specific right authorised by its source
operation while preserving no universal right the source lacked; OpenAt's
`TRAVERSE`-to-`READ` rule is the worked case (D-173, D-181).

### W-2 — Time is read, not asked for

A read-only page carrying the counter, mapped into every program, so reading
the clock never traps.

The mechanism already exists — Phase 4 made the counter the clock and the timer
an alarm — and this is that decision followed one step into ring 3. Linux does
it with the vDSO; Zircon goes further and makes the vDSO the *only* legal
syscall path, kernel-enforced, which is a hardening result worth wanting later
even if not now.

It also settles a question that is otherwise settled by accident. A clock
reachable by any program without a handle is an **ambient facility**. It grants
no external-object authority, but it does affect determinism. WASI treats
wall-clock and monotonic time as separate capabilities; Zircon treats
monotonic time as ambient. Both are defensible. What is not defensible is
arriving at one of them because a door was easier to write than a page.

### W-3 — What happens to operations in flight

This was unspecified when these application constraints were written, and it
touches reclamation directly (D-117). D-153 and D-167 now require retirement to
finish each in-flight operation with a terminal completion while the zombie
still owns the ring, quiet the queue, and only then reclaim it.

io_uring needed `IORING_OP_ASYNC_CANCEL` and retrofitting it was painful. The
decision here is narrower than a general cancel opcode — what a *retiring*
program's in-flight work does — and it is cheaper decided than discovered.

## The reservation, and the fork it does not close

`libs/elf64` already parses `PT_NOTE`, and the loader already ignores every
segment that is not `PT_LOAD`. So a vendor note namespace costs no code: name
it, define how a note is laid out, and nothing else claims the space.

What it would eventually carry is what a program *requires* — so a spawner can
decide and enqueue a grant message before the first instruction rather than
discovering requirements through a handshake. The program is still born with
one endpoint and receives each bootstrap grant through that channel; later
local derivation remains handle-authorized (D-111, D-151). Routing decided
before a program runs can be inspected; authority negotiated
from unstated requirements afterwards cannot.

**The placement is a real fork and this phase does not close it.** Every modern
system that does this put the declaration in the *package* rather than the
executable: Fuchsia's component manifests, Android's permissions, macOS
entitlements. The argument is that routing is a deployment decision and not a
build decision — the same binary should run under different policy without
being rebuilt. The argument for the image is that there is exactly one artifact
and it cannot be separated from what it claims.

ELF notes are a standard place for this class of metadata —
`.note.gnu.property` carries CET and IBT flags, `.note.ABI-tag` has done its
job for decades — so the mechanism is not the question. The question is whether
a program's requirements and a deployment's grants are one document or two, and
this project has no package format yet to put the second one in. Reserving the
namespace keeps both answers open; choosing now would close one for no gain.

## Where this sits against the field

Recorded because the claim is checkable, and because a later session should not
have to re-derive it.

| Decision | Nearest prior art | Standing |
|---|---|---|
| Completion-based ring, no thread per operation | io_uring, Windows IOCP, WASI 0.3 | Settled modern practice |
| Completions carry handles | Zircon channels, seL4 IPC cap transfer, io_uring direct descriptors | Precedented; the *combination* with a completion ring is not well explored |
| No root; every name from a directory handle | Capsicum (2010), WASI preopens, Fuchsia namespaces | Settled for capability systems; known cost to POSIX |
| Requirements declared in the image | Fuchsia `.cml`, Android manifests, macOS entitlements | The idea is mainstream; **the placement here differs from all three** |
| Copy-then-validate on the shared ring | io_uring's CVE record is the negative example | Ahead of the reference implementation, and model-checked besides (D-112, D-113) |
| Read-only time page | Linux vDSO, Zircon vDSO | Settled; Zircon's enforcement is the stronger form, deferred |

The shape overall is a Zircon-style capability model over an io_uring-style I/O
path. No shipping system is known to have both, which is a reason to write the
reasoning down and not a reason for confidence.

## How it will be checked

The three constraints are not testable as such; what is testable is their
consequence.

- **C-1**: a program with more operations outstanding than it has threads
  completes all of them. One kernel thread, several outstanding — which is the
  property the constraint exists for, stated so it can go red.
- **C-2 / W-1**: Duplicate cannot exceed the rights offered, checked the way
  D-110 checks channel transfer. OpenAt separately proves that cross-type
  `READ` derivation never restores a universal right the directory lacked.
- **W-2**: the page is mapped user-readable and nonwritable, and the
  kernel-side reader advances and agrees with the kernel clock. Compiled ring-3
  reader evidence remains Phase 7 work under D-181.
- **W-3**: a program retires with operations outstanding; each receives its
  terminal completion before ring reclamation, and the frame count stays steady
  across the collection D-117 built.

Per D-28 every one of these is broken once and watched to fail, and per
`AGENTS.md` each gets a mutation so it cannot quietly stop being able to.

## Known holes, carried deliberately

- **The ambient-authority question is settled only for time.** Randomness is
  the same question and this phase does not answer it, because nothing needs
  randomness before Phase 8.
- ~~**No general cancel.**~~ Closed by D-164 in Phase 6: a running program may
  submit `Opcode::Cancel` against an in-flight token, with a separate terminal
  completion for both the cancel and its target.
- **The note namespace is reserved and empty.** Nothing reads it, nothing
  writes it, and the placement fork is open. That is the intended state.
- **Zircon's vDSO enforcement is not adopted.** W-2 maps a page; it does not
  make that page the only legal syscall path. The hardening is real and the
  mechanism to enforce it is not free, so it waits for Phase 12 with the other
  security work.

## Decisions this phase had to make — taken 2026-08-25

All five are settled and carry D-numbers. They are left in their original
form below, with the answer and its entry against each, because the tension
each one names is the reason the answer is not obvious.

1. **Does C-1 bind Phase 6 or the project?** As a project rule it is stronger,
   and it forecloses a synchronous door someone may later want for an operation
   that only *looks* blocking. As a Phase 6 rule it is weaker and will be
   re-argued.
2. **Does a handle arriving by completion consume a slot the receiver reserved,
   or does the kernel install it?** The first makes the ring's flow control
   cover handles; the second is simpler. Getting it wrong is a denial of
   service against the handle table.

   **The receiver reserves it, at submission (D-151.)** The ring already
   admits by reservation (D-79), so this is the flow control it has rather
   than a second one — and having refused io_uring's unbounded completion
   backlog (`IORING_FEAT_NODROP`), completion-time installation has nowhere
   to put a failure. Zircon's channels are the shape chosen here: the receiver
   supplies the handle array, so the space is committed before the message
   moves. D-151 records what the io_uring record actually says, which is not
   what an earlier draft of this line claimed.
3. **Is the time page per-program or global?** One shared read-only page is
   simplest; a per-program page is what a future that wants to virtualise time
   would need. Nothing needs the second today.

   **Global (D-152.)** `005` forbids the speculative API, and one shared
   read-only page is what a vDSO is. A per-program page is what virtualised
   time would need, and virtualised time is not something this OS has decided
   to want.
4. **What does a completion for a retired program's operation do?** Dropped at
   post, refused at submit once the program is retiring, or collected with the
   zombie. The third is closest to D-117's shape and the most work.

   **Collected with the zombie (D-153.)** `LastRites`, `orphan` and `collect`
   are that machinery already, built by D-117 for the program's memory; an
   operation in flight is one more thing the zombie holds until someone with
   nothing better to do collects it. Dropping at post needs the poster to know
   the slot's generation; refusing at submit needs the submitter to lose a race
   it cannot see.
5. **Requirements in the image or in a package.** Deferred above; recorded here
   so the deferral is on the record rather than in a paragraph.

   **Still deferred, deliberately.** There is no package format to put them
   in, and `PT_NOTE` is reserved either way. Fuchsia, Android and macOS all
   chose the package, and all three had one.
