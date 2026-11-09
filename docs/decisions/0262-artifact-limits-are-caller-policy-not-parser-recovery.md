# D-262 Artifact limits are caller policy, not parser recovery

Status: Accepted, 2026-08-31. Amends D-220 and D-244; advances PW-012, PW-014, PW-018 and PW-028.

**Context.** D-244 bound exceptional records to executions, but several byte
doors still trusted allocation to fail safely. Model output was classified and
then discarded, deterministic images were hashed through generic paths, and
records or transcripts could reach whole-file allocation before their semantic
limit. A parser refusing an oversized value after allocation is not a resource
bound. Marker substrings also let a warning or longer forged record donate a
success claim.

**Decision.** Each artifact class has one explicit maximum and exact-size bit,
shared by qualification and offline replay. Metadata is checked before reading;
hashes stream through a bounded reader; reads and copies take at most the limit
plus one and remeasure afterward. Deterministic images require their exact
extent. Model evidence retains separately digested stdout and stderr for every
registered run, in registered order, and reconstructs the exact classified
bytes. Machine evidence accepts required output only as one complete canonical
severity record; dynamic values use their own exact grammar.

**Alternatives and current evidence.** A single generous bundle-wide cap was
rejected because it does not express which caller may consume which bytes.
Trusting a digest was rejected because hashing attacker-sized input can itself
exhaust the verifier. Current host fixtures cover sparse preflight, bounded
growth, size-policy mapping, transcript identity and canonical log near misses.
Retained mutations cover the metadata, post-read growth, identity, extent,
count and policy-map doors now implemented. PW-002, PW-009 and PW-028 still own
the real external executions and closing bundle; no exceptional result is
claimed here.

**Current disposition (2026-08-31).** PW-012, PW-014 and PW-018 are `DONE`:
their implementation and exact host/mutation witnesses are complete. PW-028
remains `REQUIRED` solely for its fresh external advisory execution and
replayable closing bundle.
