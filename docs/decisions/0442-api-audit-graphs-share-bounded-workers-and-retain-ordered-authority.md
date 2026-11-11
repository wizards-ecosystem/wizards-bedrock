# D-442 API audit graphs share bounded workers and retain ordered authority

Status: Accepted, 2026-10-26. Advances PW-005 and PW-595.

**Decision.** Each round runs shared graphs in ordered batches of at most four
scoped workers. Every graph retains its private target directory, one Cargo
build job, bounded child process and output, and unchanged source, compiler
and build-script authority checks. All admitted workers join before the next
batch begins. Results remain in graph order; an error stops later batches and
reports the lowest-index failure in the completed batch. The `xtask` test
harness runs alone after the shared graphs and is skipped if a shared batch
fails. Instrumentation rounds remain sequential. Scheduling changes neither the
audited graph set nor the evidence required from a graph.

The scheduler witness must observe concurrent work, the worker ceiling,
ordered results, isolated harness execution and refusal to start later batches
after an error. Mutations 2,439 and 2,440 remove the worker bound and harness
isolation independently. No measured speedup is claimed until the complete
audit has run.

**Rationale.** Serial execution of independent graphs, each already in its own
build directory, dominated the ordinary gate, while the large `xtask` harness
has a distinct memory cost, so unbounded workers would turn spare CPU into
allocation failures. A work-stealing pool could start later graphs before an
earlier failure is observed; fixed batches make failure admission
deterministic.
