# D-442 API audit graphs share bounded workers and retain ordered authority

Status: Accepted, 2026-10-26. Advances PW-005 and PW-595.

**Context.** The compiler audit repeats independent Cargo graphs in each
instrumentation round. Serial graph execution dominates the ordinary gate,
although every graph already owns a separate build directory. The large
`xtask` test harness also has a distinct memory cost, so unrestricted parallel
compilation would turn available CPU time into avoidable allocation failures.

**Decision.** Each round runs shared graphs in ordered batches of at most four
scoped workers. Every graph retains its private target directory, one Cargo
build job, bounded child process and output, and unchanged source, compiler
and build-script authority checks. All admitted workers join before the next
batch begins. Results remain in graph order; an error stops later batches and
reports the lowest-index failure in the completed batch. The `xtask` test
harness runs alone after the shared graphs and is skipped if a shared batch
fails. Instrumentation rounds remain sequential.

The scheduler witness must observe concurrent work, the worker ceiling,
ordered results, isolated harness execution and refusal to start later batches
after an error. Mutations 2,439 and 2,440 remove the worker bound and harness
isolation independently. Their exact reds, restored host checks and the full
ordinary gate are required before this implementation is complete. Scheduling
changes neither the audited graph set nor the evidence required from a graph.

**Alternatives.** Keeping every graph serial leaves independent work idle.
Unbounded workers amplify the known memory pressure. A work-stealing pool can
start later graphs before an earlier failure is observed, so fixed batches make
failure admission deterministic. Running the large harness beside other graphs
was rejected in favor of a separate final slot. No measured speedup is claimed
until the complete audit has run.
