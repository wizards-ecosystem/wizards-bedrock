# D-319 The works are a static, verified, supervised and lazily started component graph

Status: Accepted (2026-09-12), refines D-14 and D-111

**Context.** `002` pillar 4 says no daemons by default and pillar 6 says
services are supervised, not unwound; D-14 chose supervision on Theseus's
evidence; D-111 gives every program one handle. `015` slice 7 will add
process objects and exit status. `018` section 6 found the pieces proven
separately: a service database compiled and verified offline (s6-rc), typed
dependency edges with readiness (dinit), on-demand start from a declared
endpoint (launchd), a fixed task set with no fork (Hubris), and the runtime
graph as the admin view with parents paying for children (Genode, Sculpt).
Nothing ships the combination.

**Decision.** A component is one Wizzy program plus a manifest naming its
artifact, its derived requirements, the interfaces it exposes, its
dependencies with a readiness kind, its start policy, its supervision policy
and its quota. The base system's components, edges and routes are compiled at
image build into the works file and verified there: acyclic, every dependency
present, every route provided at its version, every quota inside its parent's,
every unmet requirement marked degraded rather than hidden. init builds the
supervisor from that file and does nothing else; the supervisor owns every
lifetime, applies policies and logs every failure with an incarnation. A
`lazy` component starts on the first message to a front door the supervisor
holds and may retire when idle. Quotas flow down and an over-quota child is
refused before it exists. The Works app draws the live graph, and editing a
route edits the works file, which applies when written. Log records carry
supervisor-asserted fields apart from program-written ones. `019` section 3
has the details.

**Rationale.** A base system whose component set is fixed cannot accrete
daemons, a graph verified offline fails at build time, and a graph that is
also the admin view is how sandboxing becomes legible instead of prompt-heavy.
User-installed programs stay dynamic leaves under the bench.

**Alternatives.** A dynamic service manager with units was rejected because
flexibility in the base system is how daemons accrete. Unsupervised services
with restart scripts were rejected by D-14 already.

**Consequences.** PW-700's quota accounting has its caller. Process objects,
exit status and a multi-program image are prerequisites named in `019`
section 15. The boot self-test reads the works file back.
