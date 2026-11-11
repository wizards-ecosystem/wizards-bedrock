# D-317 The shell is typed and effect-previewed, and the terminal is a record protocol

Status: Accepted, 2026-09-12. Refines D-10 and D-183.

**Decision.** The shell speaks Wizzy's expression syntax over Wizzy's values
and evaluates a bounded command sublanguage itself, single-pass and without
macro processing; larger programs compile where the compiler runs. Pipes carry
values encoded in one canonical, versioned, streaming wire format,
`wizard-wire-v1`, defined in a host-testable crate before the shell; `bytes`
is the explicit escape hatch and the annex always speaks bytes. A stage
declares its output type or declares it unknown; a consumer that needs a known
type refuses an unknown one. Before a line runs the shell shows the union of
its stages' declared requirements narrowed by the holdings it will pass, which
needs manifests, not a compiler. A pipeline is a job with a scope; cancel uses
the ring's Cancel opcode. The terminal is a session protocol whose unit is a
record (prompt, command, output chunk, status, input, resize, focus) with a
grid per command; keyboard input carries press, repeat, release and alternates;
images, text sizes, hyperlinks and typed clipboard entries are native. VT100
is a compatibility component that decodes a foreign program's bytes into the
same records inside the annex. `019` sections 7 and 8 have the details.

**Rationale.** `015` slice 8 requires a TTY protocol, a line editor, a shell
and pipelines from explicit channels with no `fork` and no inherited
descriptors, and a target-side compiler is deliberately later. Structured
shells (`018` section 4) are proven except for type inference that passes on
the first rows and fails on the stream, and their interoperation tax sits at
the byte boundary. Typed inside is what a single-language OS can afford and
what every shell user wanted from nushell without the breakage; bytes at the
boundary is YSH's exterior-first rule and D-10's translation-layer rule
applied to pipes. Records are where Warp, Ghostty and tmux are already going
one escape sequence at a time.

**Alternatives.** Byte pipes with structure at the edges were rejected because
they put the type errors where users are. A VT100 terminal as the native
surface was rejected because it would make the annex the whole system.
Inferring types from the first records was rejected on the evidence.

**Consequences.** The wire format and the session protocol are the first two
host-testable crates of Phase 7 after the runtime shim, and `015` slice 8's
TTY protocol is the session protocol. The Phase 7 console renderer is
disposable; the records are not.
