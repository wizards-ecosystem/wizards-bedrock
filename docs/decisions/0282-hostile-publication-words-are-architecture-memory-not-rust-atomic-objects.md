# D-282 Hostile publication words are architecture memory, not Rust atomic objects

Status: Accepted, 2026-09-05. Amends D-221 and D-269; advances PW-128.

**Context.** D-269 prevented safe kernel byte access from overlapping a ring's
publication words, but the publication door still constructed `&AtomicU32`
over those bytes. Ring 3 writes the same words with ordinary machine stores and
does not participate in Rust's atomic-object rules. Alignment and the correct
ordering enum therefore did not make the safe wrapper sound: a valid program
could race a Rust atomic with a non-atomic alias.

**Decision.** A checked `SharedRing` derives only an aligned integer address
inside its fixed publication partition. A crate-private x86 boundary performs
one naturally aligned dword `mov` through opaque assembly and never constructs
a Rust reference or atomic object. The assembly declares memory effects, so it
is a compiler barrier; x86 TSO gives the load and store the acquire and release
ordering required by D-221. Endianness conversion remains in the safe wrapper,
and byte-only or payload access still cannot name a publication word.

**Alternatives and proof obligation.** `read_volatile` and `write_volatile`
were rejected because volatile access is neither atomic nor a synchronization
edge. Adding compiler fences around them does not repair atomicity, while
`AtomicU32::from_ptr` recreates the same external-alias contract. Locked
instructions are unnecessary for naturally aligned 32-bit x86 accesses and
would obscure the actual protocol. Source checks require the exact two `mov`
instructions, their memory effects, complete bounds/alignment preflight and
the absence of `AtomicU32`, references and volatile publication access. Six
independent breaks made those checks red and were restored; the retained
memory-effect and volatile-access mutations plus restored release, debug and
VMware ring exercises remain PW-128's close.
