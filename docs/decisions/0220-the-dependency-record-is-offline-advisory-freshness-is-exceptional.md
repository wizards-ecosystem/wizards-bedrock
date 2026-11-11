# D-220 The dependency record is offline; advisory freshness is exceptional

Status: Accepted, 2026-08-29. Amends D-16, D-25 and D-26; closes PW-510.

**Decision.** `supply-chain/inventory.txt` is the reviewed source of dependency
and license conclusions. Every registry tuple is bound to the name, version
and checksum in `Cargo.lock`; every vendored artifact is bound to its checked
path and SHA-256. Each row says whether it reaches the runtime image or only
development, and license conclusions include non-code data such as the Noto
glyphs. A repository-owned parser, SPDX 2.3 renderer and digest checker use no
registry cache, package manager, network or new dependency. The committed
`foundation.spdx` must be byte-for-byte what that inventory renders.

The network boundary has two explicit stages. A first checkout may run
`cargo fetch --locked` to hydrate the exact locked crate archives and rustup
may install the pinned toolchain and target, as D-16 already allowed. After
that, the complete gate must pass with Cargo offline; the supply-chain check
itself works even before hydration because it reads only committed bytes.

Vulnerability freshness cannot be made offline and current at once. At every
phase close and before a public release, a deliberately exceptional RustSec
`cargo audit` run examines `Cargo.lock`; its tool version, advisory-database
revision, time and result belong in the qualification record. It is not in
ordinary `ci`, and absence is reported as absence rather than presented as a
green advisory result. A distributable image is accompanied by the generated
SBOM, the project license and third-party notices; a bare disk image is a build
artifact, not a complete distribution bundle.

**Rationale.** `Cargo.lock` names exact crate bytes but no license, Cargo
metadata misses the SIL Open Font License on the Noto glyph data the
MIT-licensed font crate embeds, no SBOM said which inputs ship, and no policy
said which state "self-contained" meant. Making a missing `cargo-audit` fail
every build would contradict D-26. Checksums, license conclusions and shipped
scope are review facts, so they belong beside the source and must remain
verifiable when the network and Cargo cache are absent. Advisory status is a
time-varying external fact, so committing a once-fresh database or silently
accepting a stale one would confuse reproducibility with currency. The split
preserves both: local CI proves exactly what was reviewed, while qualification
records when the external claim was actually refreshed.

**Alternatives.** Adding `cargo-deny`, an SPDX crate or an online service to
the ordinary gate was rejected because the checker would add the dependency
it is meant to inventory or make D-16 depend on a host install. Treating
`Cargo.lock` as an SBOM was rejected because it lacks licenses, shipped scope,
vendored inputs and the font-data license. Calling a fresh checkout offline
without first fetching locked archives was rejected as an impossible promise.
Vendoring the entire crate registry and advisory database was disproportionate
for one developer and would turn timely external data into a large stale tree.

**Consequences.** The checked inventory names 67 registry packages and all
three vendored artifacts; its manifest-derived kernel closure identifies the
three linked crates plus `BOOTX64.EFI` as the four runtime inputs. The
deterministic SPDX 2.3 document, runtime copyright notices and complete
OFL-1.1 text are committed distribution paperwork. The complete gate passes
after dependency hydration with Cargo forced offline. Mutations 220 through 226
keep a changed locked checksum, runtime scope, selected license, vendored
digest, notice identifier, SPDX bytes and an ambiguous dependency name red.
The phase-close
checklist gains one exceptional advisory result; it does not gain a
permanently installed tool or an ordinary CI stage which can fail only because
the network is unavailable.
