# D-275 Public reachability is bound to an exact compiler origin

Status: Accepted (2026-09-01), amends D-213; advances PW-519, PW-523 and PW-532

**Context.** D-213's deprecation-marker direction was sound, but its claim that
one host graph compiled all targets was not an exact root model. Target programs
and standard test harnesses have different roots, a custom harness can generate
its own `main`, and a dependency compiling included first-party text can report
a first-party-looking source span. Macros, conditional attributes, trait method
dispatch and implicit `Drop` or `GlobalAlloc` calls also contain resolved
execution which syntax alone can over-credit. Repeated compilation additionally
inherited Cargo configuration, registry source, environment tools and build
scripts which the copied source snapshot did not bind.

**Decision.** Cargo metadata v1 defines each registered pass's exact package
identities, dependency-kind closure, targets and target roots. Every registered
pass, metadata-declared example and standard harness target receives a distinct
compiler graph and build directory. Custom-harness syntax, active procedural
macros, reserved `std`/`core`/`alloc` rebinding and source shapes whose runtime
meaning is not classified fail closed. Declarative macro bodies remain opaque
except for the constrained standard forms and exact `wost::entry!` contract the
audit models.

Only a structured Cargo JSON warning with code `deprecated`, one primary span
and the exact expected package ID, target name, target kind and canonical root
source can contribute an edge. Compiler character columns map into half-open
source ranges. A classified primary retains that lexical authority when rustc
also reports deeper expansion metadata. An unclassified primary never supplies
caller evidence; it may only be discarded when one terminal immediate
expansion maps to a classified opaque callsite under the exact Cargo origin.
Missing, unclassified, non-opaque or nested external fallback refuses.

The one credited cross-target shape is the exact audited `wost::entry!`
contract. Catalog-backed validation issues authority for exactly four
first-party identities and derives each identity's exact use site from the
token-pinned expansion; raw target discovery issues no such authority. The
primary must match that identity and site. Rustc must then report one terminal
immediate expansion named exactly `::wost::entry!` at the sole kernel
invocation, and the full Cargo origin must match that kernel target. Only then
does the definition-site use receive the invocation as its caller. A normal
same-`wost` definition-site diagnostic remains opaque and cannot claim that
edge. No other dependency macro receives consumer-origin authority.

A live trait dispatch must select exactly one audited implementation; generic
or ambiguous evidence is a refusal. `GlobalAlloc` activation requires the exact
nominal global-allocator type. Implicit `Drop` activation is limited to reviewed
nominal types reached through a live exact value construction or a validated
reviewed owned-`Self` producer. That is an honest nominal reachability boundary,
not a claim that runtime execution cannot forget or wrap the value in
`ManuallyDrop`.

Original and copied source, private Cargo configuration and materialized
registry source are real-file snapshots checked throughout the compiler rounds
and before a verdict. Locked registry packages are materialized from cached
`.crate` archives whose SHA-256 matches `Cargo.lock`; the exact package/file set
and Cargo-generated file checksums are then revalidated. Every active external
package must resolve to that exact materialized crates.io root; path and Git
packages are refused. The diagnostic marker is rejected in workspace Rust,
authenticated registry files, generated build inputs, Cargo/rustc runtime
files and the selected host and kernel sysroot inputs. Those compiler inputs
are SHA-256 bound: target libraries after each round, rustc and `rust-lld`
after each graph, and the complete runtime and target sets after metadata,
pristine compilation and all observations before the verdict. Child Cargo runs
start at the filesystem root with a blank private `CARGO_HOME`, explicit
offline source replacement, stripped ambient Cargo/Rust semantics, exact
rustflags and a toolchain-only `PATH`. Only the exact kernel linker build script
and the checksum-bound reviewed dependency-build-script set are admitted, and
their Cargo output and `OUT_DIR` authority must remain identical and
marker-free between pristine and instrumented rounds.

**Alternatives and current evidence.** Rendered short diagnostics were rejected
because they carry no trustworthy package/target origin. Merging targets was
rejected because it confuses target `main` with harness roots. Trusting a
mutable directory source was rejected because source and its adjacent checksum
map can be changed together; the lock-matched archive is the authority.
Pretending stable diagnostics prove destructor execution was rejected in favor
of the explicit construction boundary.

The focused API suite passed 107/107 in both debug and release. Weakening exact dispatch selection,
removing reviewed owned-`Self` producer activation, bypassing diagnostic-origin
binding and admitting a rebound `core` root respectively made
`ambiguous_variable_receiver_dispatch_fails_closed`,
`a_reviewed_owned_self_producer_activates_drop_without_type_use_shortcuts`,
`cargo_diagnostic_origins_bind_markers_to_the_exact_package_and_target` and
`a_rebound_core_root_cannot_forge_the_approved_global_asm_macro` red before
restoration. The real entry fixture additionally reproduced all four
consumer-origin definition shapes; granting raw discovery authority, omitting
`BootInfo`, or weakening the identity/site, declaration, terminal expansion,
macro identity, callsite, target or ordinary same-`wost` opacity made its exact
witness red before restoration. Removing the registry, generated-input,
compiler-runtime, target-metadata, digest or active-package guards likewise
made their focused witnesses red before restoration. The remaining trusted
base is the Cargo command path embedded in `xtask`, stable rustc diagnostic
behavior, the snapshot-bound compiler bytes, checksum-matched archives and
reviewed build scripts. An opaque compiler input which synthesizes a marker not
present in its exact bytes, or a same-authority mutation restored byte-for-byte
between the named checkpoints, remains outside the claim; no build-script
sandbox is claimed. The real API audit, complete CI,
PW-519/PW-523 reintroduction runs and PW-532 machine matrix are not claimed by
this focused evidence.
