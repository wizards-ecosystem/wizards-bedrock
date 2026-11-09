# D-334 Paging activation is one interrupt-excluded publication transaction

Status: Accepted, 2026-09-28. Amends D-268; closes PW-127.

**Context.** D-268 implemented the activation transaction, but closure still
required current causal evidence at each publication boundary. In particular,
the proof had to distinguish a missing interrupt exclusion, premature
readiness, root publication after readiness, CR3 after publication and a
reclaimed-root read from unrelated paging or boot failures.

**Decision.** Activation remains one `Cold -> Initializing -> Ready`
transaction. It refuses reclamation before asking for the root, retains the
caller's exact interrupt state through the transaction, writes CR3 before
publishing the root and publishes `Ready` last with Release ordering. An
observer may use the root only after it sees `Ready`.

**Evidence.** On 2026-09-28 retained mutations 778 through 784 were applied
one at a time and restored exactly. Removing the ready-byte decode failed only
`every_byte_decodes_to_one_state_or_refusal`; removing the only completion edge
failed only `only_the_protocol_edges_are_admitted`. Omitting interrupt
exclusion and publishing `Ready` immediately after the cold-state claim each
failed exactly `paging activation stays unpublished under interrupt exclusion`
and `paging activation publishes its root before readiness` at 260/262.
Publishing readiness before the root failed only the root-before-readiness
witness at 261/262. Moving publication before CR3 failed only
`paging_activation_loads_cr3_before_publishing_ready`. Bypassing the reclaimed
preflight produced the complete framed panic `a reclaimed address space has no
root` followed by `panic: terminal record complete v1`, before a self-test
verdict. After exact restoration, release and debug QEMU each passed all
262/262 checks, including both activation witnesses.

**Alternatives.** Treating a green restored boot as sufficient was rejected
because it does not prove that either pause seam or the ordering contract can
fail. Treating interrupt exclusion as incidental to a single-CPU boot was
rejected because interrupt handlers are observers on that CPU. Reading a
reclaimed root before refusing it was rejected because the ownership
precondition must dominate the read itself.
