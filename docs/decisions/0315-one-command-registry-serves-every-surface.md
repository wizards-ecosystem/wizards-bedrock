# D-315 One command registry serves every surface

Status: Accepted, 2026-09-12.

**Decision.** Every action a program offers is a command record: an id in
`noun.verb` form, typed parameters, the effect set the action needs, a default
binding, a menu placement, a context predicate and, where one exists, an
inverse. The toolkit registers a program's table and exports it over a typed
interface; the bench merges every table. Window menus and context menus are
rendered from the registry, the palette searches it, the shell calls it,
scripts and agents invoke it, jump lists are its pinned entries, the
accessibility tree exposes it as actions, and the test harness drives programs
through it rather than through synthesized input. `019` section 6 has the
details, the palette and the bench key map.

**Rationale.** A desktop has at least six ways to invoke an action, and the
platforms that built them separately are now retrofitting palettes,
automation and accessibility one at a time (`018` sections 2 and 7); one
toolkit and one language let this OS pay for the unification once. One
inventory means one place for bindings, discoverability, automation and
accessibility. macOS's menu discipline (every command in a menu with its
shortcut) is the part worth keeping; rendering the menus in the window and
searching the same inventory from a palette is the Windows-leaning half.

**Alternatives.** Per-app menus plus a launcher that indexes desktop files was
rejected because it is what every platform is now replacing. A scripting
layer that synthesizes input was rejected because it is brittle, invisible to
accessibility and unauditable.

**Consequences.** The toolkit cannot draw a control that invokes an
unregistered action, and the test harness for first-party apps is written
against ids. Teach-me mode and type-to-jump (`019` sections 6.3 and 6.4) are
experiments built on the registry and carry kill criteria.
