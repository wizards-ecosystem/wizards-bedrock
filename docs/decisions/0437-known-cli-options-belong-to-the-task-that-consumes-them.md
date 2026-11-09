# D-437 Known CLI options belong to the task that consumes them

Status: Accepted (2026-10-23), qualification OPEN; advances PW-718

The parser accepted known options on unrelated tasks and silently discarded
them. In particular `clean --check` dispatched cleanup, and `fmt -- --check`
dispatched formatting. These parser outcomes were inspected without executing
either destructive or modifying command.

Track option presence separately from its value and reject misplaced options
before task dispatch. Profile options belong to build/image/run/vmware,
test-boot, boot-check and degraded-check; `--check` belongs to fmt/debug,
`--all` to clean, `--shipped`
to VMware generation/qualification, `--message-format` to check, and the `--`
passthrough separator only to run. Explicit default profiles, empty format
values and an empty passthrough tail still count as supplied options. Immediate
help handling and existing repeated-option precedence remain unchanged.

Silently ignoring known flags makes invocation mistakes look successful.
Adding a universal check mode would invent behavior those tasks do not offer.
Task-local refusal is the smallest repair and adds no execution route.

Seven parser fixtures cover the consumer matrix, empty/default presence and
existing precedence. Their retained mutations and exact pristine/red/restored
runs are required; tests invoke the parser, never cleanup or formatting.
