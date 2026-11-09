# D-01 Kernel language: Rust

Status: Verified, 2026-07-18.

**Context.** Isaac's fluent language is C++; the project is a multi-year
from-scratch OS. The question was posed as "what is absolute best practice
for modern architecture?"

**Options.** (a) C++ (fluency, SerenityOS/Managarm precedent); (b) Rust;
(c) C++ kernel with Rust later; (d) Rust kernel with C++ userspace.

**Decision.** Rust, on the stable channel for as long as possible.

**Rationale.** The only actively-maintained first-class osdev tutorial
lineage (os.phil-opp.com) and the rust-osdev crate ecosystem are Rust; the
borrow checker eliminates the bug class (UAF, data races) that is hardest
to debug in ring 0; Linux and Windows both accept Rust for new kernel
code. Post-hoc, the research pass strengthened this: Asterinas (USENIX
ATC'25) demonstrates a Rust framekernel at real-system scale. Its containment
property is verified; its percentage-based TCB and broad performance headlines
are not evidence this project relies on, and only single-core syscall dispatch
is a defensible performance summary (D-71, `010` §1). Its lesson that safety
must be designed in clean-slate is exactly this project's position. C++ remains
welcome in userspace later.

**Amended by D-183.** That last sentence now means third-party ports and
compatibility code, not first-party target software. The kernel remains Rust;
feasible first-party target-side userland is Wizzy.

**Consequences.** Isaac learns Rust alongside OS concepts (slower start,
compounding payoff). Stable-channel discipline occasionally constrains
crate choice (see D-17).
