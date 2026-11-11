# D-427 Shipped liveness requires a call from ring 3

Status: Accepted, 2026-10-23. Qualification OPEN; amends D-116 and D-426; advances PW-004.

**Decision.** A call whose saved code selector has CPL 3 sets a boot-lifetime
atomic observation in `user::call`. The idle thread emits exactly one
`user: ring 3 call observed` info record after that observation. It takes no
console lock in trap context and adds no user ABI or target program. The
existing launch records remain diagnostics. Both shipped QEMU and VMware
qualification require the new exact record; VMware admits it independently
of the boot-banner order because scheduling is not a transcript contract.

Host contracts bind the sole producer to the saved-privilege guard, the
one-shot idle reporter and both consumers, and transcript fixtures refuse a
missing, warning, duplicate or forged VMware record. Mutations 2,298 through
2,306 remove the privilege guard, observation, idle call, one-shot latch and
each consumer requirement; on 2026-10-23 each failed its exact host witness.
Fresh shipped QEMU and VMware boots and the complete current mutation sweep
remain required; no runtime result is claimed here.

**Rationale.** D-426 showed that the shipped image could print its launch and
keyboard records before halting on its first `iretq`, so neither scheduling a
user thread nor announcing the crossing proves that a user instruction
executed, and the self-test image's calls cannot qualify the separately built
shipped image. Logging directly from the trap would break D-122's lock
discipline.
