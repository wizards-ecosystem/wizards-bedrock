# D-437 Known CLI options belong to the task that consumes them

Status: Accepted, 2026-10-23. Qualification OPEN; advances PW-718.

**Decision.** Track option presence separately from its value and reject
misplaced options before task dispatch. Profile options belong to
build/image/run/vmware, test-boot, boot-check and degraded-check; `--check`
belongs to fmt/debug, `--all` to clean, `--shipped` to VMware
generation/qualification, `--message-format` to check, and the `--`
passthrough separator only to run. Explicit default profiles, empty format
values and an empty passthrough tail still count as supplied options. Immediate
help handling and existing repeated-option precedence remain unchanged. Seven
parser fixtures cover the consumer matrix, empty/default presence and existing
precedence with retained mutations; tests invoke the parser, never cleanup or
formatting.

**Rationale.** The parser had accepted known options on unrelated tasks and
discarded them, so `clean --check` dispatched cleanup and `fmt -- --check`
dispatched formatting, making invocation mistakes look successful. A universal
check mode would invent behavior those tasks do not offer; task-local refusal
is the smallest repair and adds no execution route.
