# D-180 Wizzy gets the first native system-language/runtime proof

Status: Provisional, 2026-08-26. Amends D-05's sequencing.

**Context.** D-05 names an own userland, shell, Python and document web as the
destination. D-06 makes the native ABI capability- and completion-oriented;
D-10 already confines POSIX to an adapter. Phase 5's mlibc pressure test then
did what D-74 asked without producing a runtime (D-114). The next caller can
therefore be a real compiled direct-native program rather than making the
POSIX translation layer the OS's first application model.

`wizards-ink` is a plausible fit: its implemented effects, normal-path resource
cleanup and VM-backed structured concurrency expose the questions this ABI
was built to answer; panic cleanup and parity on other backends remain open.
The local evidence snapshot is `limbwizard/wizards-ink` commit
`9035c2e8f78ee64fa1db2b24c3bfc2ff7a6c5e8e`. It was six commits ahead of that
repository's remote when recorded, so slice 1 must replace it with a reachable
pin before deciding the route. The fit is not proof. Its tracked design says
runtime capability values and a Wasm backend are future v0.3 work,
native code generation is v0.4+ work, there is no `wzi build`, and the VM is
not a `no_std` userland runtime. The OS also lacks ring-3/native operations or
contracts for channel send/receive, handle close, user-controlled mapping,
terminal I/O, directory enumeration, spawn-from-image and child status. Its
`wost`-only unsafe rule also has no place yet for a user runtime's platform
instructions and volatile ring access.
Choosing static AOT ELF as the next slice would make the highest-cost route a
premise instead of a result.

**Options.** (a) Grow the compiled Rust boundary probe into the native
`init`, shell and tools; (b) make the mlibc/POSIX layer the first real userland;
(c) wait for a Wizzy AOT backend before doing any userland work; (d) evaluate
Wizzy through a measured user-mode delivery route while preserving a
language-neutral platform contract; (e) put a Wizzy interpreter in the kernel.

**Decision.** (d), provisionally. D-116's emitted ring-3 program is already the
first direct native caller and drives its own ring. The compiled Rust program
in the early slices is the first delegation-capable boundary probe: it proves
a real build, the wire layout and Duplicate's completion-carried handle
delegation without choosing the language of `init`, the shell or the tools.
Terminal and directory I/O delegation arrive only in later slices. Wizzy is
the provisional choice for that first system-language/runtime proof.

"Native" means that a program uses handles, completion rings,
directory-relative names and bootstrap protocols directly. It does **not**
mean AOT. Phase 7 begins with a bounded comparison whose default hypothesis is
bytecode in a static user-mode Rust runner, because the VM is the execution
engine that exists. Core Wasm + `wasip1` remains D-72's first-class app format
and Phase 7 milestone; the Component Model remains only a watch item in this
repository. Wizzy's own Component Model decision conflicts with that target
and must be resolved before Wizzy can supply the D-72 module. WASI does not
substitute for the direct-native qualification. AOT is a later optimisation
unless measurement reverses that order.

The kernel stays Rust and language-neutral. The interpreter, allocator,
scheduler and language runtime stay in userland. POSIX remains the Phase 9
compatibility gateway, and the established identities of Phases 8–13 do not
move. RustPython moves from Phase 7 to the Phase 9 compatibility-runtime work;
the Python and CPython destination remains.

**Rationale.** A user-mode runner can test Wizzy against the native boundary
without waiting for an unrelated compiler backend. It also makes the
proposition falsifiable: runtime dependencies, image size, heap cost and
process density can be measured before kernel interfaces grow around them.
Preserving the language-neutral boundary means a failed Wizzy experiment still
leaves useful bootstrap, lifecycle and service protocols. Putting an
interpreter in the kernel would instead enlarge the trusted computing base and
bind privileged code to language policy.

The Rust-native alternative is the baseline, not a straw man. It reuses the
already pinned compiler, has the smallest bootstrap dependency set and remains
the fallback if the Wizzy gate fails. The boundary probe proves the platform
contract before Wizzy runs; it does not prove Wizzy's runtime, capability
representation or semantics. Choosing Rust permanently would avoid that
runtime experiment, but would also create `init`, the shell and the first tool
surface before testing the language this decision exists to test. That trade
is why the Wizzy choice is measured and provisional.

Effects are evidence for launch policy, not authority. Wizzy capability values
are not implemented yet, an image can be forged, and no manifest grants itself
a handle. The launcher chooses what to send; the kernel enforces possession and
rights. The first Wizzy-level security claim waits for a real unforgeable
runtime value rather than exposing an integer handle and calling the wrapper a
capability.

**Promotion gate.** Accepted only when a reproducibly built Wizzy `init` runs
in user mode with exactly one handle-table entry—the bootstrap channel—while
the fixed ring, time page and handleless operations are accounted separately.
It must receive a terminal handle through that channel, drive the native ring,
print, launch two Wizzy children—one successful and one deliberately failed—
observe and reclaim both, and exit with observable status. Capability values
and their private runtime constructors must exist; forged artifacts and effect
metadata must not manufacture one. The programs must agree with the host semantic
backend, and four simultaneous Wizzy processes must fit the measured memory
budget. Two clean builds from pinned source, compiler/runtime revisions and
locked dependencies must produce identical bytes, one of which matches the
artifact in the OS image. A checksum-pinned artifact is enough to keep the
normal OS gate self-contained, but proves only artifact identity, not
reproducible compilation; that gate may not depend on a mutable sibling
checkout.

**Consequences.** `015-wizzy-userland.md` records the dependency chain and
slice proposal. Phase 6's interactive VMware checks remain the entry gate.
Phase 7 adds no speculative kernel API: channel, memory, process and terminal
operations arrive with the concrete caller named by each slice. The
user-runtime unsafe boundary gets its own decision rather than being hidden in
`wost` or generated assembly. Persistent writes remain WizFS's Phase 13 work;
Phase 7 mutating tools use explicitly volatile tmpfs.
If the promotion gate fails, the language-neutral platform work stays and the
Wizzy selection is revisited without weakening W^X, ambient-authority rules or
the POSIX boundary.

**Amended by D-183.** The Rust boundary probe remains a wire-contract oracle
and integration scaffold, not a permanent native-userland fallback. D-183
makes Wizzy the settled default for feasible first-party target-side userland,
puts an earlier compiler-readiness gate in front of Phase 7 product work, and
turns the promotion gate above into a later integration qualification. A failed
qualification returns work to that gate; it does not reopen the language choice
without a new decision.
