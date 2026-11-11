# D-269 Shared memory chooses one access layout once

Status: Accepted, 2026-08-31. Amends D-221; advances PW-128.

**Decision.** An unpartitioned `Shared` allocation is consumed exactly once
into either `SharedBytes` or one checked `SharedRing<WORDS>`. None of those
authorities is `Copy` or `Clone`. The ring constructor checks alignment,
wrapping and every publication-word/payload intersection before exposing either
access API. Byte-only pages cannot mint atomic words, and ring payload methods
cannot address a publication word.

**Rationale.** Safe volatile byte methods and atomic publication-word methods
could cover the same shared allocation. Ring 3 could then access a payload byte
while the kernel treated an overlapping word as atomic, which violates the
atomic object's exclusive layout even though every individual offset was in
bounds.

**Alternatives and current evidence.** Checking overlap independently on each
access was rejected because it preserves two safe doors and makes their future
agreement the invariant. Runtime borrowing was rejected because ring 3 is not
a Rust borrower. Host exhaustion covers touching, overlapping, misaligned and
wrapping ranges. Compiler probes reject recovery or duplication of all three
authority shapes, and separate Clone mutations plus the retained overlap
mutation fail those proofs. The current real ring boot remains in the register.
