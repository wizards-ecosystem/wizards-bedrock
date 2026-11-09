# D-349 Every RAM mapping keeps its direct-map cache type

Status: Accepted, 2026-09-29. Amends D-283; closes PW-559.

**Context.** D-283 made the two shared-mapping doors refuse `NO_CACHE` and
`WRITE_THROUGH`, because a shared allocation stays reachable through the
kernel's write-back direct map. The same reasoning covers all RAM: every
byte the direct map describes has a cache type there, and a second mapping of
another type is an alias the architecture leaves undefined. `map_owned`,
`map_user` and `map_boot` still took the caller's cache flags. The framebuffer
is the one range the boot map marks write-through, and `map_boot` carried that
by convention rather than by check.

**Decision.** `libs/memory-map` gains `direct_map_cache`, which returns the one
cache type the direct map gives a range, or `None` if the range is uncovered
or mixes framebuffer and ordinary memory. `BootRegion` records that type at
`describe` and refuses a mixed range. `map_boot` maps a range only with its own
type; `map_owned` and `map_user`, which map ordinary RAM, refuse any cache
type at all. The shared doors keep D-283's rule. Device memory keeps its
separate admitted uncached path.

**Evidence.** On 2026-09-29 a `memory-map` host test checked
`direct_map_cache` for uniform ordinary, uniform framebuffer, mixed, uncovered
and reserved ranges. A machine check refuses `map_owned`, `map_user` and
`map_boot` for each cache flag and their combination with nothing installed,
maps an ordinary boot range write-back, refuses that range write-through and
maps the framebuffer range write-through. A release self-test passed 264/264.
Mutation 1,527 makes a mixed range report one type and fails the host test;
mutations 1,528 through 1,530 drop the RAM policy from the shared, owned/user
and boot doors and fail the machine check.

**Alternatives.** Letting `map_boot` keep taking caller flags was rejected
because the framebuffer's type then rests on every caller passing it. Mapping
ordinary RAM uncached on request was rejected because the write-back direct
map alias makes the result undefined, exactly as for the shared doors.
