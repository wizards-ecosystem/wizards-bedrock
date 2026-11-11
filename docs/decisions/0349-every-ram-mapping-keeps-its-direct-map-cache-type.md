# D-349 Every RAM mapping keeps its direct-map cache type

Status: Accepted, 2026-09-29. Amends D-283; closes PW-559.

**Decision.** `libs/memory-map` gains `direct_map_cache`, which returns the one
cache type the direct map gives a range, or `None` if the range is uncovered
or mixes framebuffer and ordinary memory. `BootRegion` records that type at
`describe` and refuses a mixed range. `map_boot` maps a range only with its own
type; `map_owned` and `map_user`, which map ordinary RAM, refuse any cache
type at all. The shared doors keep D-283's rule. Device memory keeps its
separate admitted uncached path.

**Rationale.** D-283 refused `NO_CACHE` and `WRITE_THROUGH` at the two shared
doors because a shared allocation stays reachable through the write-back
direct map. The same holds for all RAM: a second mapping of another cache type
is an alias the architecture leaves undefined, yet `map_owned`, `map_user` and
`map_boot` took the caller's flags, and the write-through framebuffer rested on
convention rather than a check. Mutations 1,527 through 1,530 retain the
mixed-range and per-door breaks.

**Alternatives.** Letting `map_boot` keep taking caller flags was rejected
because the framebuffer's type then rests on every caller passing it. Mapping
ordinary RAM uncached on request was rejected because the write-back direct
map alias makes the result undefined, exactly as for the shared doors.
