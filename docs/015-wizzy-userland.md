# 015 — Wizzy as the first native system-language/runtime: feasibility and Phase 7 plan

The plan extends D-116's emitted native caller into a compiled Rust program
that exercises delegation, then tests whether Wizzy can carry the native
system-program runtime without pretending its compiler backend or target
runtime already exists.

This is a **provisional plan**, governed by D-180. The kernel stays Rust. POSIX
stays a userspace adapter. What is provisional is whether Wizzy can carry the
native system programs soon enough, within a runtime small and coherent enough
for this machine. Phase 7 begins by measuring that proposition rather than by
committing the OS to an unbuilt AOT backend.

The language implementation is
[`limbwizard/wizards-ink`](https://github.com/limbwizard/wizards-ink). Its
tracked `docs/os-userland.md`, `docs/design/authority.md`, and `ROADMAP.md` are
the evidence used here, read from the clean tree at commit
`9035c2e8f78ee64fa1db2b24c3bfc2ff7a6c5e8e`. At this decision point that commit
was six commits ahead of the sibling's remote, so it is not yet a remotely
fetchable provenance pin. The relevant facts are restated below, and slice 1
must replace the local snapshot with a reachable revision before recording its
route ADR. It is never a build dependency: an ordinary checkout of this
repository must not silently build against whatever happens to be in
`../wizards-ink`, and the promotion gate pins the artifact contract again.

## What "native" means

Native describes the **platform contract**, not the instruction generator.
A native Wizzy program:

- receives authority through the bootstrap channel and handle table;
- submits native operations and correlates their completions directly;
- names files from directory capabilities, never an ambient root;
- uses the global monotonic time page exactly as D-152 defines it;
- composes processes through channels rather than descriptor inheritance;
- does not reach those facilities through POSIX, WASI, or a C ABI.

A user-mode bytecode runner can satisfy every item. An AOT ELF can satisfy
every item. A core Wasm module using `wasip1` does not: it uses a deliberately
first-class compatibility contract, valuable in its own right but not the
native ABI. "Native userland" therefore does not force "native codegen first."

The kernel must not acquire a Wizzy parser, evaluator, scheduler, allocator
policy, or type checker. It loads images, maps memory, moves messages and
handles, and will eventually report process outcomes. Language semantics stay
in userland.

## Evidence at the decision point

### What the OS has

- an ELF64 loader, separate address spaces, ring 3, guarded stacks, and W^X;
- one handle table per process, rights attenuation, and channels that transfer
  bytes and handles inside the kernel;
- a submission/completion ring with echo, uptime, duplication, timers,
  cancellation, `OpenAt`, and `Read`;
- a global read-only monotonic time page;
- NVMe, read-only FAT32, a VFS, tmpfs, and devfs;
- a spawn-only kernel model in which a child starts with one bootstrap channel;
- retirement which keeps in-flight work away from reclaimed ring pages.

### What the OS does not have

- a ring-3 channel send, receive, or channel-pair creation operation;
- a ring-3 handle-close or virtual-memory mapping operation;
- a terminal input/output capability usable by a process;
- a ring-3 directory-enumeration operation;
- a user-facing spawn operation or a way to select among executable images;
- an observable child status or process object;
- a versioned bootstrap message;
- a compiled userland crate—the shipped ELF is emitted machine code whose job
  is to test the loader and boundary;
- a decision reconciling a user runtime's platform `unsafe` with the current
  rule that `wost` is the only crate in this repository permitted to write it;
- a write path through the VFS; FAT32 remains read-only.

`init` plus a child is therefore not the next small edit. It is the far end of
a dependency chain through channels, bootstrap, memory, images and lifecycle.

### What Wizzy has

- a checked language, tree-walking interpreter, and bytecode VM held together
  by differential tests;
- enforced effects; cleanup on normal return, `try`, `break` and `continue`,
  while panic cleanup remains open; structured concurrency in the VM backend;
  diagnostics, formatting, and host tooling;
- a host authority report and launch flags over whole effects.

### What Wizzy does not have

- `wzi build` or a stable serialized program artifact;
- a separable VM artifact—the current VM retains checked compiler structures;
- a `no_std + alloc` runtime separated from the `std` compiler;
- a platform seam replacing host filesystem, environment, clock, sleep and
  console calls;
- runtime capability values—the tracked authority design schedules them for
  v0.3;
- a settled byte-buffer representation for page-sized I/O;
- a Wasm backend, scheduled for v0.3;
- a native backend, scheduled after Wasm in v0.4+.

The fit is promising, not already implemented. Effects are useful evidence
about what a program may request; they do not mint authority. A manifest is not
a security boundary, and an image cannot grant itself anything by naming an
effect. The launcher chooses which handles to send, and the kernel enforces the
rights on those handles.

## Delivery routes

| Route | Evidence now | Cost before the first boot | Standing |
|---|---|---|---|
| Bytecode in a static user-mode Rust runner | The implemented non-reference backend already agrees with the tree walker on the host | Split a small runtime from `wizc`, remove its dependency on checked AST/compiler state, define an artifact, add `no_std + alloc`, and measure the per-process footprint | **First probe, not yet the permanent format** |
| Direct Rust binaries | The pinned OS toolchain exists; no compiled userland crate does | Define and audit the user-runtime boundary, then grow only the programs actually chosen | Baseline and fallback; the first boundary probe, not a Wizzy qualification |
| Core Wasm + `wasip1` | The OS plans this under D-72; the pinned Wizzy snapshot instead commits its backend to the Component Model and calls the fork unresolved | A wasmi-class runtime and WASI host bindings here; a superseding or clarifying Wizzy ADR before using its output | Separate Phase 7 milestone; a non-Wizzy module may prove D-72, and WASI does not satisfy D-180 |
| Static AOT ELF | The OS loader exists; the Wizzy backend does not | WIR lowering, code generation, relocations, TLS, runtime ABI, object emission and linking | Later optimisation, not the bootstrap |
| Interpreter or compiler in the kernel | No supporting evidence | Enlarges the trusted computing base and couples language policy to the kernel | Rejected |

The repositories do not currently choose the same Wasm artifact. D-180 does
not silently supersede Wizzy's ADR-0026. Slice 1 records and costs the fork; a
Wizzy Wasm artifact waits for a decision in the language repository. Phase 7
may still prove D-72 independently with a non-Wizzy `wasip1` module.

The route decision is made after a bounded spike. The default Wizzy hypothesis
is a bytecode artifact inside a statically linked Rust runner ELF because it
reuses the implementation that exists. Direct Rust is the costed baseline, not
an omitted competitor. Slice 1 first supplies enough user-target and linker
scaffolding to make target sizes real, then records image bytes, private
startup pages, peak heap and retained dependencies for both routes. A bytecode
artifact that still needs `Checked`, an AST, compiler hash maps or host services
has failed extraction. A route that cannot run four simultaneous Wizzy
processes—the minimum for `init`, a shell and a two-stage pipeline—has failed
the Phase 7 density requirement. The spike records the numbers and result; it
does not answer failure by starting Cranelift early.

## Ownership of the seams

### The OS repository owns

- the platform ABI: entry state, calls, rings, handles, status values and
  bootstrap protocol;
- executable loading, process creation and lifecycle;
- terminal, filesystem, networking and device services;
- launch policy and the handles actually granted;
- the self-contained image and every boot check over it;
- Wizzy system-program sources whose behavior is part of this OS.

### The language repository owns

- Wizzy semantics, checking, effects, resources and diagnostics;
- bytecode, Wasm and native compiler formats;
- the language runtime ABI, standard library, allocator-facing runtime and
  platform interface;
- host simulation and the differential oracle;
- compiler and developer tooling.

### The handoff is shared and versioned

The artifact format, target identifier, platform ABI version, required runtime
version and effect-requirement encoding must agree. Neither repository may
infer the other's current checkout. The normal OS gate consumes committed or
vendored artifact bytes with pinned provenance and a checked digest. That
proves exactly which bytes booted; it does not prove how they were produced.

D-180's promotion gate is stronger. A separate integration task regenerates
the artifact twice from pinned Wizzy source, pinned compiler/runtime revisions
and locked dependencies, requires byte-identical output, and compares it with
the bytes the OS image consumes. The ordinary `cargo xtask ci` remains
self-contained and never changes meaning with a sibling checkout.

## Entry gate

Phase 6 is code-complete but not closed under D-18. The interactive keyboard
checks recorded in `014-storage.md` still require a person at the machine.
That human run and its dated result come before Phase 7 implementation. Writing
this design while the run is open is harmless; claiming the next phase has
started is not.

## The dependency chain

```text
measured delivery route
  -> minimal user target + compiled Rust boundary probe + repaired ABI description
  -> channel operations + handle close + versioned bootstrap
  -> terminal/directory delegation + visible hello
  -> VM mapping + runtime allocation
  -> platform seam + byte buffers + unforgeable capability values
  -> chosen Wizzy runtime + deterministic artifact
  -> process object + spawn-from-image + exit status
  -> Wizzy init + child
  -> channel creation + directory enumeration + TTY, shell and pipelines
  -> tmpfs mutation + mutating tools
```

Each arrow has a first caller. No later item is used to justify a speculative
earlier API.

## Phase 7 slices

The table is a proposal until slice 1 records the route decision. Its order is
the useful part: every slice leaves a bootable system and exposes the next
missing contract.

| Slice | Deliverable | Proof |
|---|---|---|
| 0 | **Close Phase 6.** Complete the interactive VMware keyboard rounds | Dated VMware verdict with every current check exercised |
| 1 | **Measure the runner.** Add the minimal user target/link scaffold needed for honest target measurements; extract the smallest bytecode execution closure, reject retained compiler state, inventory capability/byte-buffer blockers, and compare exact image/private/heap numbers with direct Rust and the missing AOT work; cost Wasm and record its sibling fork | A delivery-route ADR with raw numbers and a four-process pass/fail result |
| 2 | **The Rust boundary probe.** Decide and audit the user-runtime `unsafe` boundary, add a compiled Rust boundary program, and carry D-181's repaired C pressure test into a chosen shared platform-ABI source; keep the emitted loader probe | Host ABI/layout and linked-ELF checks; the emitted and compiled programs each boot in their own image matrix because image selection does not exist yet |
| 3 | **Bootstrap channels and cleanup.** Add asynchronous send/receive and handle-close operations plus a versioned bootstrap codec in a host-testable `libs/` crate | A child's handle table starts with exactly one entry; it receives bytes plus attenuated handles, closes them, and stale generations and last-reference cleanup are observed |
| 4 | **The first delegated output.** Give the child terminal and directory capabilities through bootstrap; do not add ambient handles | The Rust boundary probe receives authority, writes visible output, opens one file relative to its directory, closes both delegated handles and exits |
| 5 | **Runtime memory.** Choose a non-blocking/non-allocating trap contract or a deferred mapping path, then add only the map/unmap, allocation and stack-growth mechanisms the measured execution route calls | The chosen path never allocates or blocks in forbidden trap context; W^X survives growth, failure is reported, and repeated launch/exit holds frame counts steady |
| 6 | **The Wizzy platform, then `hello`.** Separate the platform seam, compact byte buffers and capability-first standard-library surface; make capability values unforgeable; produce the deterministic artifact only after those prerequisites | Host and target agree; forged bytecode/effect metadata cannot synthesize a capability or invoke a privileged builtin; the granted terminal value alone permits output |
| 7 | **Lifecycle.** Choose executable authority, then add a process object, spawn from an executable handle, exit status and child observation; retain the measured fixed capacity | A readable file alone cannot authorize kernel spawn or executable mapping; Wizzy `init` observes one successful and one failed child; an over-capacity spawn refuses without leaking |
| 8 | **Terminal system.** Add channel-pair creation, directory enumeration, a TTY protocol, line editing, a shell and pipelines made from explicit channels | `echo`, `cat`, `ls` and logical `pwd` compose without `fork` or descriptor inheritance; a full pipeline applies bounded backpressure without spinning |
| 9 | **Volatile mutation.** Add create/write/rename/unlink to tmpfs before exposing mutating commands | `mkdir`, `cp`, `mv` and `rm` work in tmpfs; FAT32 remains read-only and reboot discards the changes |
| 10 | **Core Wasm and `wasip1`.** Deliver D-72 independently; use a Wizzy-produced module only after the sibling Component/Core-Wasm fork is resolved | A module receives only its preopened handles and survives the same authority checks |
| 11 | **Phase close.** Audit, mutation sweep, applicable model checks, QEMU and VMware | The full gate plus a dated, fully green human VMware boot |

## Contracts forced by the slices

### Entry and bootstrap

D-182 pays the entry contract with its first caller. At the first instruction,
`rsp` names three words: entry version 1, the launch argument, and the numeric
identity of the sole bootstrap handle. A program refuses an unknown version
before interpreting the following words. The precise authority claim remains
that the handle table contains exactly one entry at launch; a scalar naming
that entry is not a second grant. The fixed ring, time page, and handleless
ring/control operations are ambient non-handle facilities and are enumerated
rather than called absent. Terminal and directory handles, service endpoints,
and future configuration arrive in the versioned bootstrap message; they are
not extra ambient entry-state fields.

The global monotonic time page is deliberately **not** a bootstrap capability.
D-152 made it one read-only page mapped into every process. Wizzy may describe
time observation as an effect, but the current kernel cannot withhold it per
process and this phase does not pretend otherwise. Wall time remains absent.

### Channels and rings

Channel traffic is data and encounters backpressure, so send and receive use
the ring rather than the synchronous call numbers sketched by the Phase 5 C
header. Receive reserves room for every transferred handle before accepting
the operation, the same rule D-151 established for completions. Message bytes
and handle fields are versioned and bounded.

The existing channel holds sixteen messages and only a receiver can wait.
Slice 3 must choose either a pending asynchronous send or an explicit
would-block result plus a waitable writable event; retry spinning is rejected.
Its machine proof fills the channel, demonstrates bounded progress when a
receiver frees space, then covers peer close, cancellation, sender retirement,
exactly-one completion, reservation return and rights attenuation. A
one-message happy path proves none of those semantics.

The runtime owns raw ring layout and volatile access. Wizzy application code
sees typed operations and completion tokens, not pointers, indices, or handle
integers. Pulling sized integers into the language is justified only by a
source-level caller; the Rust runtime needing `u32` is not one. A compact byte
buffer is a real language/runtime question because file and channel payloads
need one without boxing every byte.

### Authority and effects

The first Wizzy-level security claim waits for its capability values to exist
with a settled unforgeability story. A temporary plain integer handle exposed
to Wizzy would not be a capability abstraction and is not an acceptable
bootstrap shortcut.

Effects remain valuable before then:

- the compiler derives what a program may request;
- the image records that request for review and launch policy;
- the launcher may grant less or refuse to start;
- the kernel trusts only possession of a handle and its rights.

The derived request never causes an automatic grant. That keeps a forged,
stale or hand-built image from turning metadata into authority.

Capability values and the capability-first platform library are a language
deliverable, not a sentence in the launch gate. Constructors that wrap raw
handles or reach privileged platform builtins remain private to the audited
runtime. The artifact verifier rejects a hand-built value or opcode that would
bypass those constructors; effect metadata is never accepted as authority.

### Memory and images

The existing loader was proved with a tiny purpose-built ELF. A real compiled
userland image will force decisions about segment count, relocations, TLS,
stack alignment, zero-fill and image identity. Each extension is made against
the compiled artifact and checked by reading the linked file back. Dynamic
linking is not part of Phase 7.

If slice 1 selects the bytecode route, its runner uses `no_std + alloc`; host
compiler dependencies do not cross into the image. Its scheduler parks green
threads on completion waits and uses the existing cancellation operation only
when a real outstanding request supplies the caller. A different route records
its own equally explicit runtime boundary rather than inheriting these details.

The Phase 5 header had sketched VM mapping as a synchronous call; D-181 removed
that speculative number. Ordinary trap-context work may neither allocate nor
enter `ThreadContext` or blocking-lock paths. D-164's audited
`sleep_from_trap` scheduler yield is the existing narrow exception; it switches
through the saved frame rather than blocking an ordinary kernel context.
Page-table growth, anonymous frames and virtual-address selection can require
more. Slice 5 therefore chooses and proves an execution model—pre-reservation,
deferred ring/service work, or an explicitly re-audited trap invariant—rather
than exposing `map_user` and calling the operation complete.

The same constraint applies to teardown. Closing the last handle reference can
deallocate, and unmap/free carry flush and reclamation obligations. D-182
therefore records those sysdeps as PARTIAL rather than treating an existing
internal method as a user door that only needs a number.

### The ABI transcription is not yet the source

`userland/abi/wizard_abi.h` is a valuable Phase 5 pressure-test transcription,
not a frozen shared ABI. D-181 repaired the contradictions this audit found:
it deleted the colliding signal and speculative call numbers, corrected
Duplicate's fields and the ambient-facility claim, documented raw rights no
current object admits, and made the partial monotonic reader validate and
saturate a fenced 128-bit conversion. It also made the remaining gap explicit:
the reader has no shared, token-correlating Uptime fallback, and version 1 has
no unsolicited-event representation. D-182 then added the versioned three-word
entry block and its first consumer.

Slice 2 first chooses the canonical ABI description and how Rust, C and the
Wizzy runtime derive or check their views. It must compile or replace D-181's
source-checked clock reader without weakening its 128-bit saturation, fenced
snapshot, monotonic-only guard, page validation or explicit fallback refusal.
The first shared ring client then finishes that fallback with correlated
Uptime completion handling. Copying even a repaired transcription into several
runtimes would still duplicate the interface rather than share it.

### The second safety boundary

`wost` is currently the only crate in this repository allowed to write
`unsafe`, and it is specifically the privileged kernel framework. A user-mode
runtime nevertheless has to issue the platform gate, perform volatile accesses
to shared ring memory, and read the architecture counter. Those operations are
platform `unsafe` under D-41, but they do not belong in a privileged library
linked into ring 3.

Slice 2 must record where that second boundary lives and amend the convention
if it adds a crate here. The acceptable shapes are explicit and auditable: a
minimal user-runtime crate with its own safe-wrapper contracts and pinned
unsafe budget, or an equally pinned runtime owned and gated by `wizards-ink`.
Hiding the instructions in unaudited generated assembly, an opaque source-less
object, or an `#[allow(unsafe_code)]` user program would evade the rule rather
than answer it. A reproducible prebuilt artifact with audited source is not the
problem. Application and system-program source remains safe Wizzy code either
way.

### Lifecycle

The current kernel function named `spawn` loads one global image and returns
the far end of a bootstrap channel to kernel code. The userland operation must
instead name an executable object or file handle, create a process object, and
return observable authority to the parent. Exit status belongs to that object
or another explicitly chosen protocol; silently discarding it cannot support a
shell.

A File currently makes no `EXECUTE` distinction. Slice 7 must either introduce
a loader-minted executable object or add a real executable-file right with a
checked mapping path. Mere `READ` authority must not authorize kernel spawn or
an executable mapping. A user-mode interpreter may still consume readable
bytes as bytecode; that is authority already exercised inside the reader. The
choice is recorded before the spawn operation is exposed.

A failed child must not kill `init`, lose its status, retain handles, or write a
late completion into reclaimed memory. The existing zombie collection is the
mechanism to extend, not a parallel lifecycle.

## Terminal work before persistent writes

D-73 remains in force: Phases 7–11 use a read-only third-party disk format and
WizFS remains Phase 13. A useful terminal does not require lying about that.

- `echo` needs only the terminal.
- `cat` and `ls` use file and directory capabilities over read-only FAT32.
- `pwd` reports the shell's logical path; a directory capability has no parent
  and there is no global root from which the kernel could reconstruct one.
- create, write, rename and unlink first target tmpfs, whose contents are
  explicitly volatile.
- `cp` may copy from FAT32 into tmpfs. `mv` is an atomic tmpfs rename only;
  cross-backend movement is not promised.
- an editor may work in tmpfs but cannot claim persistence.

Package installation, saved configuration, durable updates and self-hosting
wait for a writable persistent format. Host-built, checksum-pinned programs
may still ship in the read-only image before then.

## Verification rules

| Claim | Evidence |
|---|---|
| Delivery route is viable | Measured runner size/dependencies and a recorded comparison, not roadmap intuition |
| Artifact identity is pinned | The OS image build verifies the digest of the exact bytes it consumes |
| Compilation is reproducible | Two clean builds from pinned source, compiler/runtime revisions and locked dependencies are byte-identical and match the consumed artifact |
| Bootstrap starts with one handle | The table contains only the bootstrap channel; the ambient ring, time page and handleless operations are enumerated separately |
| Runtime semantics agree | One Wizzy source passes the host oracle and the target boot |
| I/O is correlated | Multiple out-of-order-capable submissions return exact tokens, statuses and results |
| Channel pressure is bounded | A full channel makes no spinner; receive permits progress; close, cancellation and retirement complete exactly once |
| Delegated lifetime closes | Closed handles go stale immediately and last-reference resources and quota return |
| Effects do not grant authority | A requested-but-ungranted operation is refused by missing handle/rights |
| Wizzy values are unforgeable | Forged artifacts, values and effect metadata cannot reach a privileged runtime constructor |
| Read is not kernel execute | A merely readable file cannot authorize spawn or an executable mapping |
| Lifecycle is complete | Parent sees status; child failure is isolated; frames and handles return |
| Shell is non-POSIX-native | A pipeline is explicit channels with no inherited descriptor table or `fork` |
| Writes are honestly volatile | tmpfs mutations work and FAT32 mutation is refused |

Protocol, encoding and policy arithmetic belongs in `libs/` and runs on the
host. Machine behavior belongs in `kernel/src/selftest.rs`. Claims about the
tree belong in `xtask` tests. Every new check is first broken and observed red;
every machine check gets a repeatable mutation whose boot names that failure.

## Promotion and rejection gates

D-180 becomes Accepted only when the Phase 7 system-program proof:

1. uses an artifact set regenerated by two clean builds from pinned source,
   compiler/runtime revisions and locked dependencies, with identical bytes
   matching the OS image artifacts;
2. runs its Wizzy `init` and child program in user mode without a host
   interpreter or compiler;
3. starts `init` with exactly one handle-table entry, its bootstrap channel,
   while the ring, time page and handleless operations are accounted as
   ambient facilities;
4. receives a terminal handle through that channel;
5. drives the native ring through the runtime and prints;
6. starts one successful and one deliberately failing child, observes both
   statuses, and reclaims both;
7. agrees with the host semantic backend and exits with observable status; and
8. runs four simultaneous Wizzy processes—the `init`, shell and two pipeline
   stages used to set the route threshold—within the measured memory budget.

Failure is informative. If the runtime cannot be separated without importing
the compiler, if capability values remain unavailable, or if the measured
footprint is not viable, the OS keeps the language-neutral bootstrap and
lifecycle work and revisits the Wizzy selection. It does not weaken W^X, add
new ambient handles or facilities, move an interpreter into the kernel, or
disguise POSIX as the native API to save the experiment.

## Deliberately later

- persistent writes and package installation, with WizFS in Phase 13;
- a native compiler, formatter, LSP or package manager running on the target;
- self-hosting;
- dynamic linking and general C/POSIX compatibility;
- networking, which remains Phase 8;
- the Component Model, still D-72's watch item;
- graphics and the document-web integration phases.

The claim is not that Wizzy is the first code to cross the boundary—D-116's
emitted probe did that, and the compiled Rust probe deliberately reaches
delegation first. The stronger result is that a real Wizzy runtime can carry
native system programs across the same boundary without POSIX, WASI or new
ambient handles; D-152's global time-page exception remains explicit.
