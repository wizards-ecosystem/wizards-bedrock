# D-183 The Wizzy compiler gate precedes native-userland product work

Status: Accepted, 2026-08-26. Amends D-01, D-05, D-20 and D-180.

**Decision — privilege fixes the first language boundary.** The kernel and all
first-party code that executes in kernel context remain Rust. That includes the
architecture boundary, drivers, filesystems and network stack. Required
platform assembly remains contained by the existing Rust abstractions; no
Wizzy compiler, interpreter, allocator, scheduler or language runtime enters
the kernel.

**Decision — feasible first-party target userland is Wizzy.** Every
first-party program, service, library, tool or application authored to run on
this OS outside the kernel defaults to Wizzy. This includes `init`, service
management, the shell, native commands, administration and repair tools, the
compositor, native toolkit code and first-party applications. "Default" is an
admission rule, not a preference: a different implementation language needs
one of the narrow exceptions below, a recorded reason and an exit condition.

"Feasible" means the qualified toolchain and runtime can express and ship the
required target behavior without weakening safety, reproducibility or the
platform contract. Schedule pressure and missing convenience libraries do not
make a component infeasible; a missing language or runtime capability must be
named and evidenced.

The exceptions are:

- the smallest audited runtime, entry, ABI and instruction shims needed to run
  Wizzy safely on the target;
- host-only build, image, test and verification tooling;
- third-party libraries and ports, plus compatibility layers including mlibc's
  POSIX gateway;
- foreign runtimes whose purpose is to execute another format or language,
  including the WASI interpreter and Python runtimes.

These are bootstrap, interoperation or verification boundaries, not alternate
native userlands and not precedents for first-party product code. Project-owned
code inside an exception migrates to Wizzy when the language can express it
without weakening safety, reproducibility or the platform contract. Unmodified
third-party source remains foreign by definition; its first-party adapters
still carry that migration obligation. The exceptions do not independently
waive the closed-gate sequencing below.

**Decision — compiler readiness is an early gate.** The gate is **CLOSED**
until one evidence record establishes all four conditions together:

1. A reachable Wizzy repository revision and every dependency are pinned; an
   unpublished local commit is not a pin.
2. One documented non-interactive command consumes source and emits a
   versioned, standalone program artifact that the target runner can identify
   and reject when unsupported.
3. The target runtime is separated from the compiler and its AST behind a
   documented platform seam, and is suitable for the OS's `no_std + alloc`
   environment. The compiler does not ship merely to execute its output.
4. Two clean builds from the recorded revision, dependencies and toolchain
   produce byte-identical artifacts.

The artifact may be bytecode or native code. Requiring AOT would confuse an
optimisation with deliverability; carrying compiler or AST internals into the
target runtime would confuse a development environment with a system runtime.
Opening the gate is recorded by a decision with the pins, commands, hashes and
seam named, rather than inferred from a successful developer checkout.

**Decision — a closed gate stops product implementation.** While it is closed,
Phase 7 product implementation and every later first-party target-side userland
product slice stop. Work is in scope only when it is one of:

- kernel correctness work;
- a language-neutral platform prerequisite with a concrete, named Wizzy caller;
- integration specification or scaffolding needed to exercise the artifact,
  runtime seam or gate; or
- work in `wizards-ink` that advances the gate itself.

**Current amendment (D-184/PW-601).** In this repository, the third item means
specification and validation of the versioned handoff only. Consumer
scaffolding remains stopped until both a pinned draft artifact and its runtime
seam exist. The fourth item names externally owned work in the separate
`wizards-ink` repository; it never authorizes compiler implementation here.

Host tooling is allowed only in support of that work. The compiled Rust
boundary probe may remain as an oracle and may validate a language-neutral wire
contract, but it does not grow into `init`, a shell, a service or a product
tool. A proposed prerequisite with no concrete Wizzy caller waits; "needed by
userland eventually" is not a caller.

**D-180's promotion gate becomes integration qualification.** Once compiler
readiness is open, D-180's Wizzy `init`, bootstrap-only authority, terminal,
child lifecycle, capability-construction, semantic-parity and process-density
requirements remain the definition of a successful OS integration. They no
longer decide whether Wizzy is the native-userland language. The Rust baseline
remains a test oracle, not a permanent fallback. A failed qualification is fixed
in Wizzy, its runtime or the language-neutral platform seam; if it invalidates
compiler readiness, the early gate closes and work returns to the bounded list
above. Substituting a Rust native userland or revisiting the language requires a
new decision that explicitly supersedes this one.

**Rationale.** Rust has the mature freestanding and safety machinery needed in
ring 0; putting an experimental language runtime there would enlarge the trusted
computing base and bind privilege to language policy. Above that boundary,
Wizzy is the product choice, and making the compiler prove basic delivery before
product work is the only ordering that prevents both a speculative OS contract
and a completed alternate userland.

The ABI stays language-neutral because authority and wire layouts are platform
facts, not compiler features. The exceptions keep bootstrapping honest: mlibc,
CPython, a WASI interpreter and host `xtask` do not become Wizzy by declaration.
Narrowing them and requiring migration of first-party portions prevents
"temporary shim" from becoming a second native application model.

"Language-neutral" named two different properties. The ABI must not privilege
one source language: ports, runtimes and future compilers all consume the same
handles, rings and bootstrap protocols. The product still needs a language
identity, and leaving it revisitable after building `init`, a shell or tools
lets schedule pressure turn temporary scaffolding into the userland by default,
then charges a rewrite to the language it was meant to test. D-04 puts drivers,
filesystems and the network stack in the kernel, so D-08's WizFS is Rust too;
the shared name is not a language exception.

The gate exists because the D-180 snapshot had no reachable pin,
non-interactive build command or target-suitable separated runtime. A full
Wizzy `init` is too late to discover that no reproducible program can be built;
building the OS userland in Rust while waiting is too early to make Wizzy the
system language.

**Alternatives.** D-180's measured choice with a permanent Rust fallback was
rejected because the fallback can consume the product surface before the
measurement happens. Building a Rust userland and rewriting it later was
rejected as two implementations and an ABI shaped by the disposable one.
Waiting for AOT was rejected because a reproducible standalone bytecode artifact
is sufficient to begin integration. Freezing all OS work was rejected because
kernel correctness, concrete language-neutral prerequisites and compiler work
retire real risk without pre-empting the product language. Making the ABI
Wizzy-specific or putting its interpreter in the kernel was rejected because
either would turn a userland policy into platform lock-in.

**Consequences.** The compiler-readiness gate is closed at this decision. The
Phase 7 proposal may refine specifications and the runner seam, but its product
slices do not begin until a later decision opens the gate. No Rust `init`, shell
or native tool is accepted as schedule-preserving interim work.

D-10 and D-74's mlibc work remains a compatibility exception, not native
userland and not an independent reason to cross the closed gate. D-72's WASI
runtime remains a foreign-runtime exception. WizFS remains Rust because it is
in the kernel; its first-party target-side repair and administration tools
default to Wizzy once the gate opens. Where a roadmap or the D-180 slice
proposal admits broader work, this decision controls the sequencing.

An `xtask` host check reads `015`'s canonical language and gate rows, requires
the contributor instructions, current-facing README, vision, architecture and
platform charter to point at this decision, and rejects D-180's retired
provisional/fallback phrases in those files. It was broken once by changing the
gate row to **OPEN** and failed on that exact disagreement before the row was
restored. This is a repository-claim check, not a machine behavior mutation.
