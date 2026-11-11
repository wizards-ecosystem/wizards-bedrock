# D-393 API audit reduces compiler evidence while it streams

Status: Accepted, 2026-10-12. Amends D-391 and D-347; advances PW-595.

**Decision.** Audited Cargo compilations keep compact structured JSON but move
their stdout onto a dedicated bounded stream. Every complete JSON record is
parsed fail closed as it arrives. Marker diagnostics are reduced immediately
to the package, target, code, level, primary span and expansion fields already
consumed by the audit; selected test-harness artifact records are reduced to
their exact package, target kind and test-profile verdict; other valid Cargo
records are discarded. No raw stdout is retained on disk. One JSON record has
an 8 MiB ceiling. Retained marker evidence is limited to 64 MiB including each
fixed record and every owned string or path byte; a separate source-derived
count ceiling of `64 MiB / size_of::<MarkerDiagnostic>()` fixes the vector's
maximum spare-capacity overhead without imposing a lower arbitrary workload
limit. Stderr separately keeps the existing 64 MiB ceiling, the compilation
keeps its ten-minute deadline and the Windows job still owns the whole child
tree. Reader failure, malformed or unterminated JSON, marker-count or
retained-byte overflow, a reader panic, a stderr overflow or a reader that does
not close after tree termination all fail the compilation.

A source contract requires the compiler graph to use
the streamed path rather than whole-output capture. Parser fixtures must prove
that unrelated valid records can exceed a small test line ceiling in aggregate
without retention, while a single oversized or unterminated record is refused
before unbounded allocation. Marker fixtures must prove both retained-count and
retained-byte overflow are refused. Process fixtures must prove success, parser
failure, stderr overflow and post-exit reader closure. Mutation 1,866 removes
the per-record bound and mutation 1,867 removes the aggregate retained-byte
bound; each must fail its exact unit.

**Rationale.** After D-391 the expanded `xtask` CLI harness again crossed the
64 MiB whole-child capture ceiling: the authoritative diagnostic fields stay
small, and the growing cost is accumulating every unrelated Cargo message and
unused field before parsing. Raising the limit repeats D-391's rejected
amplification, splitting antichain rounds multiplies already-expensive
whole-workspace compiler passes, and filtering raw lines before parsing could
discard malformed output or accept lookalikes, and stripping only `rendered`
still stores many unused spans and children. Typed reduction after parsing
keeps exactly the evidence the verdict uses.
