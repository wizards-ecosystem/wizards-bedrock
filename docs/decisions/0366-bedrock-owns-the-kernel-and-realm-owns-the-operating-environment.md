# D-366 Bedrock owns the kernel and Realm owns the operating environment

Status: Accepted (2026-10-07)

**Context.** Isaac requested the full rename of wizards-os to wizards-bedrock,
the kernel/userland separation and takeover of both projects' handoffs. The
local folder already has its new name. GitHub now retains the existing history
under wizards-bedrock; Realm remains its separate repository.

**Decision.** Bedrock owns Rust kernel mechanisms, wost, hardware drivers,
kernel ABI, diagnostic boot images and their qualification. Realm owns the
compiler-readiness decision, application-facing platform semantics, both
Bedrock/Linux adapters, Wizzy services, shell, desktop, toolkit, compatibility
ports, launch policy and product images. Ink owns language semantics, compiler,
execution runtime and its generic platform interface. D-183's language choice
and D-184's finite kernel register remain in force.

Move the current product specifications to Realm with source revision and
SHA-256 provenance. Existing numbered paths become migration pointers where
needed by historical links. Bedrock's 015 document retains only the kernel
handoff contract and a CLOSED integration admission condition; Realm's
compiler decision does not silently authorize kernel changes. Each project
builds and checks its own committed inputs without reading a sibling tree.

The uncompiled Phase-5 C ABI/sysdep pressure test is a kernel contract fixture,
not a product userland. Move it from userland/ to fixtures/abi-pressure/ and
retain every ABI and body-state witness. The future executable POSIX adapter
belongs to Realm. Synthetic handoff and ring-3 machine-code witnesses remain
Bedrock tests; they are not language runtimes or fallback applications.

Rename display strings, repository metadata, generated diagnostic image/VM
names and SBOM identity together. Stable wizard-kernel/wost crate names,
wizard-platform-v1, x86_64-unknown-wizard and wire identifiers are compatibility
contracts, not marketing names; changing them requires a separate ABI decision.
Historical decisions, sealed receipts and original artwork retain provenance.

**Qualification.** The migration supersedes Q's source freeze, not its unmet
proof obligations. Q's interrupted sweep and failed VMware round remain
historical. The successor needs its own ordinary CI before commit and fresh
source-bound exceptional qualifications before foundation close. Neither the
compiler gate nor a PW row opens or closes because a name changed.

**Alternatives.** A forked kernel would discard the continuity of almost all
implemented work. Sibling path dependencies would prevent independent builds.
Duplicating current userland specifications would create competing authorities.
Deleting the ABI pressure test would discard kernel contract coverage.

**Validation required.** Kernel CI, retained mutation-anchor coverage, Realm
standalone document/input checks, brand export validation and a current-name
inventory. Review brand proofs at README and icon sizes before distribution.
