# 013 - The application platform (Phase 6, slices 1-3)

**Ownership after D-366 (2026-10-07).** Bedrock owns the kernel mechanisms
and ABI described here. Product userland, graphics policy, compatibility ports
and application delivery belong to [Realm](https://github.com/wizards-ecosystem/wizards-realm).
The original phase numbers and dated observations remain historical; they do
not create a second current product backlog in Bedrock.

What a program is allowed to ask for, decided before the first deferred ring
operation was written.

Phase 5 fixed the handle table and the queue ABI, both the kind that get
frozen by their first caller. Phase 6 supplied three distinct pressures: Timer
was the first deferred operation and proved ring wait; Duplicate first proved a
completion could carry authority, with OpenAt as the motivating namespace
caller; Read was the first storage/data caller. This file, a charter per D-19
written before its code, makes that freezing a decision rather than a side
effect: three constraints, three pieces of work, and one reservation.

**It is not a phase of its own.** It was drafted as "Phase 6.5", to run after
the filesystem. That ordering was rejected (D-149): storage is the first
external-I/O subsystem that eventually needs blocking service, so building it
first and extending the ABI afterwards would leave `read()` with two doors.
These became **Phase 6's first three slices**.

References: `010-research-notes-3.md` section 2 (the async ABI and its four
hard problems), section 3 (capabilities and the handle table), section 5 (the
POSIX bill), section 12 (what must be decided before code); `011-userspace.md` throughout, and
`fixtures/abi-pressure/README.md` for the sysdep finding that motivates constraint C-3.
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
| The program's stack is the boot-time arena, so sixteen is a cap on concurrency | D-117; `011` close | **Not here.** It is old Phase 7's, and a shell reaches it immediately |
| `xtask/src/elf.rs` transcribes the ELF header rather than using `libs/elf64` | D-105; `011` close | **Not here** |
| A program receives exactly one handle at spawn and must negotiate the rest | D-111 | Reserves metadata that lets a spawner decide and enqueue bootstrap grants before the first instruction; does not build delivery |

## The principle

**The ABI must never require one kernel thread per outstanding operation.**

The rest of the file is downstream of this, and it does not depend on anything
above the kernel. Sixteen concurrent programs is today's cap (D-117), so
thread-per-operation is already the wrong shape *for this machine*; an event
loop or a server holding many connections dies against a door that can only
block. The principle says not to undermine the completion-based ring when a
synchronous door would be quicker to write.

io_uring is the prior art and the warning: its model is right, and its
implementation's opcode sprawl, kernel worker threads and revalidation of
shared memory accumulated enough exploitable bugs that Google disabled it by
default in Android and ChromeOS in 2023. The current ring has seven opcodes and
still no worker pool, and Phase 5 refused revalidation by construction
(`queue.rs`: the copy is what is used, and a second read would be the double
fetch the design refuses).

## Three constraints on Phase 6

These are rules, not code.

### C-1: No blocking-only door

Every operation that can wait has a ring form. Operations that cannot wait may
stay direct. At adoption the examples were the clock, `exit` and `tcb_set`;
D-166 later made the clock a page load, while Exit and TcbSet remain direct.

Storage supplied the first external-device operation that must eventually
defer rather than poll inside `RingEnter`, which is why the rule landed before
its VFS. Timer became the actual first deferred operation and proved the wait
contract.
The current polled Read has the right ring shape but has not yet paid the
asynchronous device-service half.

### C-2: A completion may carry a handle

Duplicate was the first implementation and proof; OpenAt is the namespace
operation whose useful result is newly derived authority. Without it, channels
can move a handle and the ring cannot, so every authority-producing operation
detours through a channel: two round trips and two mechanisms for one idea.
The near prior art is io_uring's *direct descriptors* (`IORING_OP_ACCEPT`
and `IORING_OP_OPENAT` with `sqe->file_index` set); the model for the
*capability* half is Zircon, where a channel message carrying handles is the
ordinary case.

### C-3: The filesystem has no root

Every name resolves from a directory handle the program was given. No global
root appears, in Phase 6 or later.

This is pillar 1. Native OpenAt now exists, so D-181 records `sys_open` as
PARTIAL rather than absent: ring 3 still cannot receive the first delegated
directory, and a POSIX root and current directory may be constructed only from
directory handles the process was delegated. It is written here because a VFS
is where an ambient root grows out of convenience. Capsicum reached openat-only
in 2010, and WASI's preopened directories and Fuchsia's per-process namespaces
arrived at the same answer independently.

The cost is known: a libc built on this cannot serve `open("/etc/foo")` to a
process nobody gave a root to. That is the point rather than a defect.

## Three pieces of work

### W-1: Completions carry handles

Same-object Duplicate must attenuate the way channel transfer does (D-110). A
cross-type producer may instead derive only a type-specific right authorised by
its source operation while preserving no universal right the source lacked;
OpenAt's `TRAVERSE`-to-`READ` rule is the worked case (D-173, D-181).

### W-2: Time is read, not asked for

A read-only page carrying the counter, mapped into every program, so reading
the clock never traps: Phase 4's counter followed one step into ring 3, as
Linux's vDSO does.

A clock reachable by any program without a handle is an **ambient facility**:
it grants no external-object authority, but it does affect determinism. WASI
treats wall-clock and monotonic time as separate capabilities and Zircon treats
monotonic time as ambient; both are defensible, and arriving at one because a door was easier to write than a page
is not.

### W-3: What happens to operations in flight

D-153 and D-167 require retirement to finish each in-flight operation with a
terminal completion while the zombie still owns the ring, quiet the queue, and
only then reclaim it (D-117). This is narrower than a general cancel opcode,
which io_uring had to retrofit as `IORING_OP_ASYNC_CANCEL`.

## The reservation

`libs/elf64` already parses `PT_NOTE`, and the loader ignores every segment
that is not `PT_LOAD`, so a vendor note namespace costs no code. It was
reserved for what a program *requires*, so a spawner can decide and enqueue a
grant message before the first instruction rather than discovering
requirements through a handshake. The program is still born with one endpoint
and receives each bootstrap grant through that channel; later local derivation
remains handle-authorized (D-111, D-151). Routing decided before a program runs
can be inspected; authority negotiated from unstated requirements afterwards
cannot.

The placement was a real fork this phase left open: whether a program's
requirements and a deployment's grants are one document or two, with no
package format yet to hold the second. D-316 resolved it: requirements go in
the package manifest, grants are the bench's per-program document, and the note
namespace stays empty.

## Where this sits against the field

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

## How it is checked

The constraints are checked through their consequences.

- **C-1**: a program with more operations outstanding than it has threads
  completes all of them. One kernel thread, several outstanding, which is the
  property the constraint exists for, stated so it can go red.
- **C-2 / W-1**: Duplicate cannot exceed the rights offered, checked the way
  D-110 checks channel transfer. OpenAt separately proves that cross-type
  `READ` derivation never restores a universal right the directory lacked.
- **W-2**: the page is mapped user-readable and nonwritable, and the
  kernel-side reader advances and agrees with the kernel clock. Compiled ring-3
  reader evidence remains old Phase 7 work under D-181.
- **W-3**: a program retires with operations outstanding; each receives its
  terminal completion before ring reclamation, and the frame count stays steady
  across the collection D-117 built.

Per D-28 every one of these is broken once and watched to fail, and per
`AGENTS.md` each gets a mutation so it cannot quietly stop being able to.

## Known holes, carried deliberately

- **The ambient-authority question is settled only for time.** Randomness is
  the same question and this phase does not answer it, because nothing needs
  randomness before old Phase 8.
- ~~**No general cancel.**~~ Closed by D-164 in Phase 6: a running program may
  submit `Opcode::Cancel` against an in-flight token, with a separate terminal
  completion for both the cancel and its target.
- **The note namespace is reserved and empty.** Nothing reads it and nothing
  writes it; D-316 keeps it so.
- **Zircon's vDSO enforcement is not adopted.** W-2 maps a page; it does not
  make that page the only legal syscall path. The hardening is real and the
  mechanism to enforce it is not free, so it waits for old Phase 12 with the other
  security work.

## Decisions this phase had to make, taken 2026-08-25

1. **Does C-1 bind Phase 6 or the project?** **The project (D-150).** A phase
   rule is weaker and would be re-argued; the project rule forecloses a
   synchronous door someone may later want for an operation that only *looks*
   blocking.
2. **Does a handle arriving by completion consume a slot the receiver reserved,
   or does the kernel install it?** Getting it wrong is a denial of service
   against the handle table.

   **The receiver reserves it, at submission (D-151.)** The ring already
   admits by reservation (D-79), so this is the flow control it has rather
   than a second one, and having refused io_uring's unbounded completion
   backlog (`IORING_FEAT_NODROP`), completion-time installation has nowhere
   to put a failure. Zircon's channels are the shape chosen here: the receiver
   supplies the handle array, so the space is committed before the message
   moves. D-151 records what the io_uring record actually says, which is not
   what an earlier draft of this line claimed.
3. **Is the time page per-program or global?** **Global (D-152.)** `005`
   forbids the speculative API, and one shared read-only page is what a vDSO
   is. A per-program page is what virtualised time would need, and virtualised
   time is not something this OS has decided to want.
4. **What does a completion for a retired program's operation do?** **Collected
   with the zombie (D-153.)** `LastRites`, `orphan` and `collect` are that
   machinery already, built by D-117 for the program's memory. Dropping at post
   needs the poster to know the slot's generation; refusing at submit needs the
   submitter to lose a race it cannot see.
5. **Requirements in the image or in a package.** **Resolved by D-316.**
   Requirements are derived into the package manifest and grants are the
   bench's per-program document; the note namespace stays empty. The deferral
   read: there is no package format to put them in, and `PT_NOTE` is reserved
   either way; Fuchsia, Android and macOS all chose the package, and all three
   had one.
