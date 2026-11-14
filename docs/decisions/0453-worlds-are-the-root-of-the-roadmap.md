# D-453 Worlds are the root of the roadmap

Status: Accepted, 2026-11-14. Amends D-28 and D-452; records the second
version of `docs/021-roadmap.md`.

**Context.** D-452 fixed a position and a phase order on 2026-11-09. On
2026-11-14 the owner reviewed it against a second round of research: the
market for isolation hosts, systems and verification work from 2022 to
2026, kernel speed and size, language and kernel co-design, isolation
inside one process, kernel support for language runtimes, durable
execution, and candidate uses. The research found the design close to the
current frontier and the order late on the choices that are hardest to
add afterwards: proofs, the clock, the extension checker and durability.
It also found two plans that contradicted the owner's direction: native
laptop drivers, and one Linux guest beside a system call runner.

**Decision.**

1. The root idea is the world: a nesting group of processes with
   everything it owns, its memory and the kernel's records about it, its
   handles, rings, clock and randomness, budget and child worlds. The
   kernel creates, freezes, copies, moves, rewinds and destroys a world as
   one object. Five rules hold from Phase 7: a world owns everything it
   uses; everything that enters it passes through the kernel; it has only
   what it was handed; the walls are real and proven; the steady data path
   needs no kernel entry. Realm is the world manager on top; Linux and
   other software run inside worlds.
2. A world never touches the hardware clock or random-number instructions
   unless granted. Each world runs normal (a kernel clock page, the
   default), recorded (served and logged) or raw (granted, neither
   replayable nor timing protected).
3. The frame allocator is cache-colour aware from the first version,
   kernel frames included; colouring is enforced by per-world policy.
4. Phase 7 also fixes flat descendant sets, copy-on-write clonable arenas,
   fresh randomness on copy, per-world seeds for handle numbers and
   layout, single-use reply rights, bounded running time in the extension
   form and spare capability fields for future hardware.
5. Proof is by milestone. Each core piece carries a short written
   statement of what must be true; Verus proofs are written and checked
   at the milestones `021` marks. Between milestones a phase keeps one
   witness, a boot smoke check and a focused test per real bug. D-28's
   mutation per new check ends. The extension checker is among the first
   proof targets. The CSPRNG moves to Phase 7, which serves randomness to
   every world.
6. The native ABI stays language neutral, and no ABI feature is accepted
   unless Wizzy's types express it exactly. Bedrock and Wizzy meet at a
   published core contract each implements; neither depends on the other.
7. Linux boxes follow worlds as Phase 12. A box is booted once, frozen and
   cloned copy-on-write per tenant; worlds of one owner may share a box and
   strangers never do. D-452 item 15's runner becomes an open decision,
   capped at about eighty system calls and built only if clone
   measurements show a box is too heavy.
8. Native drivers exist only for NVMe, the server NICs and virtual
   devices. All other hardware runs in driver worlds, Linux guests behind
   the IOMMU. This replaces the native laptop drivers of D-452's Phase 16.
9. Phase 13 of D-452 becomes three phases, extensions, replay and
   schedulers, and a durable-worlds phase follows replay: a standby holds
   the world's snapshot and live input log, and a reply leaves only after
   the standby holds its inputs.
10. Worlds of one owner sharing a process stay a research track, with the
    attack suite first and a deciding measurement against one process per
    world. Uses stay out of the roadmap.

**Consequences.** `021` carries the new position, choices, order and
mechanism table. Hardware with user interrupts and protection keys is
not bought now; those speed checks wait.

**Alternatives.** Keeping D-452's order put the clock, the checker and the
proofs after the code they constrain, where each would be a retrofit.
Native laptop drivers cost a driver family per device class for no gain
on the server path. A system call runner first would grow into the
compatibility layer D-452 ruled out, while cloned boxes already make a
Linux tenant cheap.
