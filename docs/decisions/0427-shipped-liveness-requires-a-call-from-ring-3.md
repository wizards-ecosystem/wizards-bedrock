# D-427 Shipped liveness requires a call from ring 3

Status: Accepted, 2026-10-23. Qualification OPEN; amends D-116 and D-426; advances PW-004.

**Context.** D-426's VMware diagnosis showed that the shipped image could
print its launch and keyboard markers before halting on its first `iretq`.
Neither scheduling a user thread nor announcing the crossing proves that a
user instruction executed. The self-test image's successful calls cannot
substitute for the separately built shipped image.

**Decision.** A call whose saved code selector has CPL 3 sets a boot-lifetime
atomic observation in `user::call`. The idle thread emits exactly one
`user: ring 3 call observed` info record after that observation. It takes no
console lock in trap context and adds no user ABI or target program. The
existing launch records remain diagnostics. Both shipped QEMU and VMware
qualification require the new exact record; VMware admits it independently
of the boot-banner order because scheduling is not a transcript contract.

**Proof.** Host contracts bind the sole producer to the saved-privilege guard,
the one-shot idle reporter and both consumers. Transcript fixtures refuse a
missing, warning, duplicate or forged VMware record. Persistent mutations
remove the privilege guard, observation, idle call, one-shot latch and each
consumer requirement. On 2026-10-23 mutations 2,298 through 2,306 each failed
their exact host witness, followed by byte-exact restoration and passing
source/producer contracts plus all 32 VMware host tests. The latter need the
normal Windows token for their symlink fixtures. Fresh shipped QEMU and VMware
boots and the complete current mutation sweep remain required; no runtime
result is claimed here.

**Alternatives.** Reusing the pre-entry announcement was rejected because the
observed VMware failure printed it. Logging directly from the trap was
rejected under D-122's lock discipline. Requiring the self-test verdict in a
shipped transcript would qualify another image and was rejected.
