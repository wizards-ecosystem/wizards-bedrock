# D-19 Method and voice

Status: Accepted, 2026-07-18.

**Decision.** *Design-first*: every subsystem gets a numbered doc in `docs/`
before implementation, citing references (OSDev wiki, Osdev-Notes, Intel
SDM, phil-opp as reference, never followed linearly). *Theme*: the
wizard/magic identity lives at the product level (app names, shell
personality, boot messages such as the serial banner "The Wizard's OS
awakens"); code uses conventional professional naming. Docs are numbered in
creation order and never renumbered.

**Rationale.** Docs-before-code makes every subsystem resumable after months
away. Theme in code ages badly.

**Amended by D-314.** Product names are plain instrument words; apps are
named by function. The wizard identity stays in the ecosystem brand, boot
messages and copy voice; code stays conventional.
