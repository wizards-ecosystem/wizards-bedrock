# D-280 One address space mints one binding for one process lifetime

Status: Accepted, 2026-09-05. Amends D-117 and D-123; advances PW-539.

**Decision.** A user `AddressSpace` may mint one `Binding`; later binds refuse
without changing the root, active root or scheduler capacity. The sole
production bind remains `user::spawn_transaction`. It installs `task::Bound` in
`BootstrapAuthority::Bound`, retirement exchanges that value for `Released`,
and `AddressSpace::reclaim` consumes the exact release evidence.

A source inventory excludes self-test callers, requires that single production
bind and pins the complete bound-to-released-to-reclaimed chain. A separate
machine witness keeps the first binding, attempts a second, and then proves
exact release, reclamation and frame restoration.

**Rationale.** `Binding` was linear after construction, but that did not prove an
`AddressSpace` could construct it only once. The current user-copy concurrency
limit also depends on one production task owning each user space, while neither
the caller inventory nor that lifetime was mechanically visible.

**Alternative and proof obligation.** Relying on a non-`Clone` binding was
rejected because two calls could still mint two distinct linear values. A prose
caller count was rejected because it would silently rot. The exact inventory
and machine witness are implemented, and guard-removal and second-owner
mutations are registered. PW-539 remains `READY` until those breaks are
executed and a restored release boot passes.
