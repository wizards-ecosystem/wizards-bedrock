# D-258 APIC deliverability is one shared typed decision

Status: Accepted, 2026-08-31. Advances PW-121.

**Decision.** `DeliverableVector` is the sole pure constructor for this
relationship. It accepts a source only when both it and the configured
spurious vector use an external-interrupt frame and are distinct. Command and
timer encoders consume that type, so neither can restate or omit the rule.
Hardware wrappers return their existing typed configuration errors when the
authority cannot be constructed, before issuing a register write.

**Rationale.** Self-IPI and one-shot timer planning each checked the rule
separately, and only the command path exhausted the vector domain and retained
a mutation, so the timer's duplicate predicate could be deleted while the
recorded PW-121 witness stayed green.

**Alternatives and evidence.** Adding a second timer mutation was rejected
because it would preserve two sources of truth. Embedding the spurious value
in each register encoder was rejected for the same reason. Host tests exhaust
all 65,536 source/spurious pairs and separately walk all 256 timer sources.
Deleting the shared distinctness guard makes the fake self-IPI write and fails
its exact test; that break remains in the mutation registry, and current
machine closure remains in PW-121.
