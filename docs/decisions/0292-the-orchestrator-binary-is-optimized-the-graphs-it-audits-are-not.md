# D-292 The orchestrator binary is optimized; the graphs it audits are not

Status: Accepted, 2026-09-06. Amends D-17 and D-213.

**Decision.** `Cargo.toml` declares an `xtask` profile that inherits `dev`
and sets `opt-level = 3`. Only the `cargo xtask` alias selects it. Host tests,
the audited graphs, the mutation workspace and every kernel build keep the
profiles they had, and the tool's own debug assertions and overflow checks
stay on because the profile inherits them. The audit's discovery stage fell
from more than ten minutes to about thirty-five seconds.

**Rationale.** The API audit hashes a 119 MB materialized registry and parses
the whole workspace with `syn` many times per gate; built with the plain `dev`
profile, one audit ran for more than two and a half hours without finishing.
Optimizing the `dev` profile for the orchestrator package instead made every
audited test harness and every host-test rebuild in the mutation sweep carry
optimized codegen, which moved the cost rather than removing it.

**Alternatives.** A per-package `dev` override was measured first and
rejected for the reason above. Building the tool with `--release` was
rejected because that profile enables LTO and would make every tool edit
cost a full relink.
