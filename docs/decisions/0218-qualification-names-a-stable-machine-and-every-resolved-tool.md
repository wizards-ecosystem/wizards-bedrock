# D-218 Qualification names a stable machine and every resolved tool

Status: Accepted, 2026-08-29. Closes PW-509.

**Decision.** Every QEMU runner uses the explicit `pc-q35-9.2` machine type.
The stable hardware shape is pinned in source; the evidence record continues
to bind the QEMU executable and firmware bytes which implement it. Rust stays
pinned by `rust-toolchain.toml`, with the resolved compiler and Cargo verbose
identities recorded alongside that file's digest and `Cargo.lock`.

Evidence format v2 adds fixed-shape identities for Java and VMware's `vmrun`:
availability, normalized version output, executable length and SHA-256 are
present whether or not the optional tool exists. Absence is the literal value
`unavailable`, not a missing field. The verifier compares availability before
the remaining bytes, so a host lacking a tool used by the recorded
qualification names that dependency and both states exactly. Changed versions
or executables name their exact differing field.

One resolver owns each exceptional path. TLC prints Java's full identity and
executes that resolved path. VMware generation probes only `vmrun`, never the
GUI or a VM; it prints the CLI's identity and fully quoted path when present,
and otherwise produces the portable VM directory with an explicit unavailable
diagnostic. Version probes have a five-second deadline and reap a timed-out
child, so environment discovery cannot become an unbounded qualification job.

**Rationale.** D-216 still passed `-machine q35`, whose meaning changes when
QEMU advances (it resolved to `pc-q35-11.1`), `model-check` ran whichever `java`
was on `PATH`, and `vmware` printed a bare `vmrun` even where Workstation is
installed but not on `PATH`. A versioned QEMU machine is the one stable input in
this set; recording what an alias meant after a run would document drift rather
than prevent it. Java and VMware cannot reasonably be vendored into this Rust
workspace, but a fixed record makes their absence and substitution
machine-checkable without making them prerequisites for ordinary CI or local
release evidence. Recording the executable as well as its self-reported version
catches repackaged bytes carrying the same display string.

**Alternatives.** Keeping `q35` and relying on the QEMU digest was rejected
because the same source would silently request a new hardware ABI after a tool
upgrade. Requiring Java and VMware for `ci` or every evidence build was rejected
for D-18/D-26's existing reason: missing exceptional tools must not teach a
contributor to skip the local gate. Vendoring a JVM or a VMware appliance would
greatly widen the repository and still not license or reproduce the host
hypervisor. Absolute installation paths were rejected because they identify a
directory layout, not tool bytes, and make an otherwise portable manifest
host-specific.

**Consequences.** The resolved Java path ran TLC 2026.08.21 with Microsoft
OpenJDK 21.0.5 and zero errors. VMware generation identified `vmrun version
1.17.0 build-24583834`, printed its usable absolute command and started no VM.
Persistent host mutations keep `q35`, `vmrun` usage text as identity, an
unavailable tool's schema and the availability-first comparison red. A
committed format-v2 evidence
bundle and QEMU boot still have to be regenerated before this successor commit
is called qualified; the exceptional results above prove tool resolution, not
the phase-close PW-001 through PW-004 evidence on a final commit.
