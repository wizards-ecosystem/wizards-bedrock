# D-258 APIC deliverability is one shared typed decision

Status: Accepted (2026-08-31), advances PW-121

**Context.** Self-IPI and one-shot timer planning independently checked that a
source used an architectural external-interrupt frame and differed from the
configured spurious vector. Only the command path exhausted the vector domain
and retained a mutation. The timer's duplicate predicate had sampled tests and
could be deleted while the recorded PW-121 witness stayed green.

**Decision.** `DeliverableVector` is the sole pure constructor for this
relationship. It accepts a source only when both it and the configured
spurious vector use an external-interrupt frame and are distinct. Command and
timer encoders consume that type, so neither can restate or omit the rule.
Hardware wrappers return their existing typed configuration errors when the
authority cannot be constructed, before issuing a register write.

**Alternatives and evidence.** Adding a second timer mutation was rejected
because it would preserve two sources of truth. Embedding the spurious value
in each register encoder was rejected for the same reason. Host tests exhaust
all 65,536 source/spurious pairs and separately walk all 256 timer sources.
Deleting the shared distinctness guard made the fake self-IPI write and failed
its exact test; restoration passed 84 unit tests and one doctest. The same
break remains in the mutation registry, and current machine closure remains in
PW-121.
