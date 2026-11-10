# D-01 Kernel language: Rust

Status: Verified, 2026-07-18.

**Context.** Isaac's fluent language is C++; the project is a multi-year
from-scratch OS. The question was "what is absolute best practice for modern
architecture?"

**Options.** (a) C++ (fluency, SerenityOS/Managarm precedent); (b) Rust;
(c) C++ kernel with Rust later; (d) Rust kernel with C++ userspace.

**Decision.** Rust, on the stable channel for as long as possible.

**Rationale.** The only maintained first-class osdev tutorial series
(os.phil-opp.com) and the rust-osdev crates are Rust. The borrow checker
removes the bug class hardest to debug in ring 0 (UAF, data races). Linux
and Windows both accept Rust for new kernel code. The research pass later
strengthened this: Asterinas (USENIX ATC'25) shows a Rust framekernel at
real-system scale. Its containment property is verified; its percentage TCB
and broad performance headlines are not evidence here, and only single-core
syscall dispatch is a defensible performance summary (D-71, `010` §1). Its
lesson, that safety must be designed in clean-slate, is this project's position.
C++ remains welcome in userspace later.

**Amended by D-183.** That last sentence now means third-party ports and
compatibility code, not first-party target software. The kernel remains Rust;
feasible first-party target-side userland is Wizzy.

**Consequences.** Isaac learns Rust alongside OS concepts (slower start,
compounding payoff). Stable-channel discipline sometimes constrains crate
choice (see D-17).
