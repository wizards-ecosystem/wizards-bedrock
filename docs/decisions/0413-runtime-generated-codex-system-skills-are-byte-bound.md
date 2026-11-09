# D-413 Runtime-generated Codex system skills are byte-bound

Status: Accepted, 2026-10-15. Amends D-412 and D-406.

**Context.** After D-412 admitted the complete effective-config response, the
qualified Codex app-server deterministically materialized its bundled system
skills below the otherwise empty private Codex home. The old all-skills-absent
check rejected that runtime output before any access advisory or Deep scan
started. Two clean materializations had the same single `.system` root, 71
entries, 314,050 file bytes and canonical tree digest.

**Decision.** The private home may have no `skills` directory before Codex first
starts, or exactly the qualified runtime-generated system-skills tree after it
starts. The qualifier bounds depth, entry count, each file and total bytes;
rejects links, reparse points, special files and non-UTF-8 names; hashes entry
kind, length-delimited normalized path, file length and file digest; and repeats
the tree inventory after hashing. Rules, plugins, memories and all user skills
remain absent. The generated tree is checked after each Codex process as part
of the existing private-home invariant.

**Proof.** A host fixture admits absence and one exact generated tree, then
rejects one additional user-skill file. Mutation 2,068 accepts every measured
identity and fails
`runtime_generated_system_skills_are_exact_and_user_skills_stay_closed`.

**Alternatives.** Deleting the generated tree between Codex processes was
rejected because it fights deterministic runtime state and creates an extra
destructive transition. Accepting `.system` by name alone was rejected because
its executable scripts and instruction text are part of the worker input.
