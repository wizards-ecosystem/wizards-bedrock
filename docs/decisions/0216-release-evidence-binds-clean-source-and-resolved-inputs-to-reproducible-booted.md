# D-216 Release evidence binds clean source and resolved inputs to reproducible, booted bytes

Status: Accepted, 2026-08-28. Closes PW-508.

**Context.** The local gate proved ten ordered properties, image construction
read the kernel back from its filesystem, and D-25 pinned vendored binaries.
Those were separate facts. No one record said which clean source tree, Cargo
lock, compiler, emulator, firmware, storage namespace, kernel and disk image
participated in one passing qualification. Rebuilding in the ordinary target
directory also reused enough state that two successes were not evidence of
reproducibility.

An evidence file committed beside the source cannot name its own commit and
hash without a self-reference loop. A timestamped report would change when
nothing relevant did. A successful gate transcript would still not establish
that the image whose hash it prints is the image the shipped-kernel runner
booted.

**Decision.** `cargo xtask evidence` starts and ends with the same clean Git
commit and tree and refuses modified or untracked source. Kernel builds now
always pass `--locked`. The task snapshots `Cargo.lock`, `rust-toolchain.toml`,
Rust and Cargo versions, the resolved QEMU executable and version, both UEFI
firmware inputs, every vendor listing and payload, and the deterministic NVMe
namespace. Every QEMU runner uses that same resolved installation. The NVMe
builder now compares complete content rather than reusing any file of the
expected length.

After the typed local gate passes, the task removes only four fixed children
under `target/evidence/`, builds the shipped release in two fresh private Cargo
target directories, and requires the kernel ELFs and disk images to agree in
both length and SHA-256. It copies both pairs into the bundle, boots the exact
path-A copy, requires the liveness challenge and settlement, and hashes that
copy again after QEMU exits. Only then does it write `wizard-evidence-v1`.

The manifest is a sorted, timestamp-free key/value record under ignored build
output. It carries source and input identities, the fixed relative artifact
paths, lengths and hashes, every gate-stage result, the artifact-identity
result and the exact-artifact boot result. `cargo xtask verify-evidence` does
no build and contacts no network. It rejects malformed, duplicate, missing or
unknown fields, requires every result to be `pass`, re-resolves and rehashes
the current clean source and installed inputs, re-verifies vendor completeness,
and independently hashes all four bundled artifacts.

**Rationale.** The manifest is useful only if the authorities it joins are the
ones execution used. One QEMU resolver prevents the recorder and runners from
quietly naming different firmware. Two erased target directories distinguish
reproduction from an incremental rebuild. Booting the bundled copy, then
rehashing it, closes the last gap between “an image was produced,” “these bytes
were recorded,” and “a shipped kernel was exercised.”

Keeping the bundle outside Git avoids the commit/hash loop and keeps two
64 MiB disk images out of source history. Omitting time and absolute paths
makes identical inputs render identical evidence. The commit and tree retain
the historical identity; a later checkout can verify the bundle only when it
matches that identity.

**Alternatives.** Building twice in one Cargo target was rejected because it
mostly proves cache reuse. Recording only logs was rejected because prose does
not bind an artifact. Committing the manifest was rejected because it either
lies about its source commit or requires an unending sequence of successor
commits. A container or virtual-machine image for every host tool is wider
than this row and would still need firmware and hypervisor policy; PW-509 owns
the remaining pin-versus-record decisions. Signing was rejected because the
project has no release key or external verifier whose trust decision would
make a signature meaningful.

**Consequences.** On clean commit `b88e752`, the complete gate passed with
1,472 compiler-resolved API items, 188 host unit tests (one ignored helper),
and 212/212 machine checks in both release and debug. Independent target paths
produced identical 3,462,408-byte kernel ELFs at
`082e09c7b00a43318896abe01bc66921de2ebcc8c3575d9adbd9a3062628945f`
and identical 67,108,864-byte disk images at
`4daa81dd59618330843dbdafe4b7154d3ab0d807e19af8181fc6eb136b2221dc`.
The exact bundled path-A image booted, took the injected keyboard interrupt,
settled and retained that digest. A separate offline verifier invocation
accepted the manifest and bytes.

Removing lock enforcement, accepting a same-sized foreign namespace,
accepting duplicate or inexact schema fields, equating artifacts by size,
accepting a failed result, accepting dirty source, changing field order, or
trusting a recorded artifact digest each failed only its exact host check. All
nine transformations were restored and persist, bringing the mutation
registry to 213. This record covers the self-contained local gate and shipped
QEMU artifact; it does not turn the deliberately separate VMware, TLC or full
mutation qualifications into CI, and it does not close PW-509's remaining
environment recreation policy.
