# D-01 Kernel language: Rust

Status: Verified, 2026-07-18.

**Decision.** Rust, on the stable channel for as long as possible.

**Rationale.** The borrow checker removes the bug classes hardest to debug
in ring 0 (use after free, data races), and Asterinas shows a Rust
framekernel containing unsafe code at real-system scale (D-71, `010` §1).
Its lesson, that safety must be designed in clean-slate, is this project's
position. C++ remains welcome in userspace later.

**Amended by D-183.** That last sentence now means third-party ports and
compatibility code, not first-party target software. The kernel remains
Rust; feasible first-party target-side userland is Wizzy.

**Consequences.** Stable-channel discipline sometimes constrains crate
choice (D-17).
