# D-413 Runtime-generated Codex system skills are byte-bound

Status: Accepted, 2026-10-15. Amends D-412 and D-406.

**Decision.** The private home may have no `skills` directory before Codex first
starts, or exactly the qualified runtime-generated system-skills tree after it
starts. The qualifier bounds depth, entry count, each file and total bytes;
rejects links, reparse points, special files and non-UTF-8 names; hashes entry
kind, length-delimited normalized path, file length and file digest; and repeats
the tree inventory after hashing. Rules, plugins, memories and all user skills
remain absent. The generated tree is checked after each Codex process as part
of the existing private-home invariant. Mutation 2,068 accepts every measured
identity and fails
`runtime_generated_system_skills_are_exact_and_user_skills_stay_closed`.

**Rationale.** The app-server deterministically materializes its bundled
`.system` skills below the empty private home, which the old all-skills-absent
check refused. Deleting the tree between processes would fight deterministic
runtime state and add a destructive transition, and accepting `.system` by name
alone would admit unmeasured executable scripts and instruction text that are
part of the worker input.
