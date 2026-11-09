# D-318 The bench is a strip with snap semantics and a complete rail

Status: Accepted (2026-09-12), amends D-11

**Context.** D-11 chose an own compositor over a Wayland-shaped protocol and
named a terminal as the first client, and said nothing about how windows are
arranged. The owner's answer for the window model was a creative combination
of a scrollable strip and floating with snap, leaning towards the strip, for
a keyboard-first desktop. `018` found the strip to be the first tiling model
that never resizes what is being read, stacks to be a compositor primitive
worth having, Windows' snap fractions and taskbar to be what its users will
not give up, and the taskbar's five-year restoration to be the anti-example.

**Decision.** Each output is an infinite horizontal strip of columns; a column
is a stack of windows shown as tabs; widths snap to a third, a half, two
thirds and full; a new window opens at natural width beside the focused column
and never resizes it; a bay is a named strip segment recalled from the rail;
floating is one key away with Mosaic placement and the same snap fractions,
and may be set as the default for new windows. Workspaces stack per output and
there is exactly one organising hierarchy. The rail is complete on the first
day: labelled and ungrouped by default, any edge, pinned slots, jump lists
from the registry, a strict background-program area with no pixmaps, visible
when a column is filled. The compositor draws all chrome to the numbers in
`020` section 9, with 0 px corners in the strip and 8 px floating, a trust
label and a kill switch for a hung window; focus is click-through. Session
restore is a compositor primitive from the first release; the greeter runs the
bench's own code path; a safe mode boots with every user layer disabled. The
privileged parts of the protocol are separate interfaces granted by capability
to bench components. `019` section 9 has the details.

**Rationale.** The strip's one rule is what makes keyboard-first tiling
livable, and keeping the snap fractions, the rail and click-through focus
whole is what makes it Windows-leaning rather than a tiling lifestyle. Floating
as a mode of the same shell rather than a second shell is the fusion the owner
asked for.

**Alternatives.** Floating-first with a tiling mode was rejected as the
default because it makes the strip's rule an option nobody discovers.
Tiling-first with automatic layouts was rejected because automatic layouts
resize what is being read. Activities or spaces beside workspaces were
rejected on KDE's evidence.

**Consequences.** D-11's graphics direction is amended to name the shell
model and the compositor-drawn chrome; its terminal-first client order stands.
The compositor needs restore tokens, the label region and the capability-gated
privileged interfaces before the first app.
