# D-19 Method and voice

Status: Accepted, 2026-07-18.

**Decision.** *Design-first*: every subsystem gets a numbered doc in
`docs/` before implementation, citing references (OSDev wiki,
Osdev-Notes, Intel SDM, phil-opp as reference — never followed
linearly). *Theme*: wizard/magic identity lives at the product level
(app names, shell personality, boot messages — e.g. the serial banner
"The Wizard's OS awakens"); code uses conventional professional naming
throughout. Docs are numbered in creation order and never renumbered.

**Rationale.** Multi-year projects survive on re-enterable context;
docs-before-code makes every subsystem resumable after months away.
Theme-in-code ages badly; theme-in-product is charm.

**Amended by D-314.** Product names are plain instrument words and apps are
named by function; the wizard identity stays in the ecosystem brand, the boot
messages and the copy voice, and code stays conventional.
