# D-334 Paging activation is one interrupt-excluded publication transaction

Status: Accepted, 2026-09-28. Amends D-268; closes PW-127.

**Decision.** Activation remains one `Cold -> Initializing -> Ready`
transaction. It refuses reclamation before asking for the root, retains the
caller's exact interrupt state through the transaction, writes CR3 before
publishing the root and publishes `Ready` last with Release ordering. An
observer may use the root only after it sees `Ready`.

**Rationale.** D-268 implemented the transaction, but closure needed causal
evidence at each publication boundary that distinguishes a missing interrupt
exclusion, premature readiness, root publication after readiness, CR3 after
publication and a reclaimed-root read from unrelated paging or boot failures.
Retained mutations 778 through 784 each reach their exact witness or causal
panic, including `paging activation stays unpublished under interrupt exclusion` and
`paging activation publishes its root before readiness`.

**Alternatives.** Treating a green restored boot as sufficient was rejected
because it does not prove that either pause seam or the ordering contract can
fail. Treating interrupt exclusion as incidental to a single-CPU boot was
rejected because interrupt handlers are observers on that CPU. Reading a
reclaimed root before refusing it was rejected because the ownership
precondition must dominate the read itself.
