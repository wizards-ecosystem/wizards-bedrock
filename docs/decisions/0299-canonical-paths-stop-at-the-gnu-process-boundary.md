# D-299 Canonical paths stop at the GNU process boundary

Status: Accepted, 2026-09-10. Amends D-208; closes PW-031.

**Context.** The next clean mutation attempt reached its pristine host
baseline, where every Rust build failed in MinGW's linker. The isolated
workspace correctly supplied its canonical Windows target path, whose
`\\?\` verbatim prefix preserves filesystem identity. Cargo passed that path
through to the GNU linker, which parsed the prefix as a root-relative path and
reported every generated object as missing. Repeating from the thirteen-byte
checkout root `C:\wq55a30dd` produced the same failure, ruling out path length.

**Decision.** Filesystem validation, containment, copying, journaling and
artifact lookup retain canonical paths. The API auditor's existing
child-process path adapter moves into the shared Cargo driver, is narrowed to
verbatim disk and UNC paths, and becomes the one boundary used by ordinary
builds, contract probes, mutation host tests and the audit itself. Other
Windows prefixes and every non-Windows path pass through unchanged. The
private target remains the same directory and the path loses no absolute
component; only syntax the GNU tools cannot consume is removed.

Weakening canonicalization throughout the mutation workspace was rejected
because subprocess compatibility cannot choose source or state identity.
Depending on a shorter checkout was rejected by the short-root reproduction.
A relative target was rejected because its meaning would then depend on each
subprocess working directory instead of the already-confined build authority.

**Proof.** A command-construction test failed with the verbatim disk path under
the old boundary and passes with its ordinary equivalent. A second test does
the same for a verbatim UNC path. The moved disk-path defect remains the
existing mutation 1,048; replacing the UNC conversion arm with pass-through
behavior reddened its exact test before restoration and remains mutation
1,359. The complete current sweep remains PW-001.
