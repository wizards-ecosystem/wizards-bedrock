# D-226 The Wizzy handoff is an exact route-neutral envelope

Status: Accepted, 2026-08-29. Closes PW-600.

**Decision.** `libs/wizzy-handoff` owns an allocation-free parser for the
`wizard-wizzy-handoff-v1` envelope. A record carries, in one canonical ordered
key/value form:

- the `x86_64-unknown-wizard` target and `wizard-platform-v1` aggregate
  platform ABI;
- exact artifact, runtime and effect-encoding version tokens;
- a sorted, duplicate-free set of effect requirements;
- one HTTPS repository and exact lowercase Git identities for source,
  compiler and runtime, plus a SHA-256 dependency-lock identity; and
- the artifact's canonical decimal length and lowercase SHA-256 digest.

The OS-owned target and platform identifiers are fixed constants. The three
Wizzy-owned version tokens are not guessed here: the verifier accepts only the
exact values its integration caller supplies. Unknown format versions, fields
and mismatched target or ABI values are refusals. Effect names are metadata
returned to launch policy; accepting the record never mints a handle or
authority.

There is intentionally no artifact kind, runner, load address, path or
delivery-route field. The envelope identifies and authenticates opaque bytes;
WZ-002 and WZ-003 still own what those bytes mean and which runtime consumes
them. Adding or changing a field requires a new envelope version rather than
teaching an old reader to guess.

`handoff/conformance-v1.wzh` is a committed conformance fixture beside an
opaque payload. Its artifact/runtime/effect tokens use the reserved `fixture.`
namespace and its provenance identities are syntactic fixture pins, so it
makes no claim that a Wizzy artifact exists. An `xtask` repository test hashes
the adjacent payload and passes the result plus explicit expected versions to
the pure parser. Neither parsing nor the fixture consults the network,
environment or a sibling checkout.

**Proof.** Host tests accept the fixture only with the SHA-256 of the adjacent
bytes, require platform v1 to remain entry, ring and time-page v1, and refuse
an unknown envelope, an unknown, duplicate or reordered field, non-canonical
values, every target and ABI mismatch, and an identity where only length or
digest agrees. Mutations 243 through 250 keep those witnesses red.

**Rationale.** D-183 requires a versioned standalone artifact and separable
runtime before product integration, and `015`'s agreement on artifact format,
target, platform ABI, runtime and effect encoding had no machine-readable
record. Loader or runner code now would freeze the delivery-route guesses
PW-601 stops; prose alone would leave the first integration to invent version
and provenance rules. Exact caller-supplied tokens let the future image build
pin the compiler-owned sides of the seam without this repository claiming to
know them early. One aggregate platform version lets a producer reject the
native entry/ring/time contract as a whole; their internal version fields
remain the finer wire checks. Canonical ordering, spellings and encodings make
the record byte-stable and reviewable. Recording both length and digest avoids
treating a hash string as the identity of bytes no consumer actually read.

**Alternatives.** Choosing bytecode, ELF or Wasm in the record was rejected
because the readiness evidence does not support that decision. Reading
`../wizards-ink` was rejected because it would make ordinary CI depend on
ambient state. Hard-coding speculative Wizzy artifact/runtime versions was
rejected because those versions belong to WZ-002 and WZ-003. A permissive
TOML/JSON document was rejected for this small contract because ignored or
duplicated fields and multiple equivalent serializations are exactly what a
version envelope must exclude.

**Consequences.** Gate opening must supply real, reachable provenance and
explicit supported version tokens in this format; the conformance fixture
cannot satisfy WZ-001 through WZ-004. A future need for multiple repositories,
another digest, or new metadata is an envelope-v2 decision, not an optional v1
field. PW-601 remains stopped until the real artifact and runtime seam exist.
