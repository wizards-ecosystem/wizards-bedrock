# 019 - The userland: the bench, the works and the ledger

What The Wizard's OS is like to use, decided before any of it is written.

**Standing.** This is a specification. The compiler-readiness gate in
`015-wizzy-userland.md` is **CLOSED** under D-183, and the
[pre-Wizzy foundation register](016-pre-wizzy-foundation.md) remains the only
backlog the OS repository may work from under D-184. Nothing here reopens either
decision: the design assumes a complete Wizzy so that the userland is designed
once rather than shaped by whichever compiler feature exists first, and it is
implemented only after the gate opens (D-314). The two ledgers in section 15
are the design's handoff: what the kernel must provide and which Wizzy
milestones it stands on, each with the caller that justifies it.

The kernel's shape is the user model. A program is born holding one thing
(D-111), names files from directories it was given rather than a root (D-171,
`013-application-platform.md` C-3), talks through channels and a completion
ring rather than a descriptor table (D-06), and is supervised rather than
unwound (D-14). Every earlier desktop hid properties like these behind a
familiar surface. This one shows them, because they are the good part.

References: `018-research-notes-5.md` throughout, in particular sections 0, 3.4,
3.8, 4.4 and 8; `002-architecture.md` pillars 1 to 6 and its graphics
direction; `011-userspace.md` on the handle table and the queue; `013` on the
platform constraints and the placement fork; `014-storage.md` on names,
rights and cursors; `015` on the Phase 7 chain, entry contract and integration
qualification; the language repository's authority design, roadmap rows and
`os-userland.md` for the Wizzy side; `.github/BRAND.md` for names and voice.
Outside the project: Polaris and the powerbox, Access Control Gadgets, Yee's
principles of secure interaction design, Sculpt's component graph, Plan 9's
plumber and rio, Haiku's queries, niri, COSMIC's stacks, the Fluent 2 chrome
specification, YSH's exterior-first rule, the kitty keyboard and graphics
protocols, AccessKit, dinit and s6-rc, the Design Tokens format. Every
citation is in `018`.

---

## 1. The concept, and the principles that follow from it

A keyboard-first instrument bench. Authority is the desktop's visible
material: every window shows what it holds, and giving is a gesture, never a
prompt. The command line and the graphical shell are one typed command
registry, and the system says what a command may do before it runs. The
system's own structure (services, holdings, places, history, settings) is a set
of documents the user can open, diff and edit.

Nine principles carry the rest of the document. Each has a section, and each
has a row in the check table in section 17.

1. **One handle at birth, no ambient authority, no root.** The design uses the
   kernel's model as the user model instead of hiding it (D-111, D-171).
2. **Designation is authorization.** Pickers, drops and plumbing mint grants.
   There are no permission prompts. Requirements and grants are two documents.
3. **One command registry.** Every action is a named, typed, effect-annotated
   command; menus, the palette, the shell, scripts, jump lists, accessibility
   and tests consume the same registry.
4. **Typed inside, bytes at the boundary.** Pipes carry Wizzy values over
   explicit channels; VT100 and POSIX live only in the compatibility annex.
5. **Never resize what the user is reading.** A strip of columns and stacks,
   with Windows snap fractions, a complete rail, per-output workspaces and
   session restore.
6. **One toolkit, one style source.** Tokens are an OS API; appearance,
   contrast and motion are three independent preferences.
7. **The works are static, verified, supervised and lazy.** No daemons by
   default (`002` pillar 4), parents pay for children, the graph is the admin
   view.
8. **Honest state.** Volatile and durable are visibly different; snapshots are
   places.
9. **Conform at the boundary, innovate inside** (`010` section 5). Ports run in
   the annex with the conventions they expect; nothing native pretends to be
   POSIX.

## 2. Names

Plain instrument words, following `.github/BRAND.md`: interfaces are capable
instruments. Code keeps conventional professional naming (D-19); these names
are for people.

| Name | What it is |
|---|---|
| the bench | The desktop session and graphical shell as a whole, and the process that runs it |
| the bench key | The one modifier the shell owns (Super on a PC keyboard); every shell shortcut is bench plus something |
| the strip | The per-output scrolling layout of columns and stacks |
| the rail | The taskbar |
| the palette | The command palette and launcher |
| places | The directory capabilities the user has named: Home, Projects, this USB |
| the deck | The strip's home column: places, bays, saved queries, recent items |
| a bay | A named segment of the strip, recalled from the rail |
| the ledger | Holdings and the grant log, and the app that shows them |
| the works | The live component and service graph, and the app that shows it |
| wires | A running pipeline drawn as a graph |
| plumbing | The content-routing rules, and the service that applies them |
| the clip | Clipboard history |
| the log | The structured journal |
| the annex | The compatibility container for foreign programs |
| the wire format | The typed, streaming encoding every first-party tool speaks |
| the session protocol | The record stream between a shell and its terminal |

Apps are named by function: Terminal, Editor, Files, Viewer, Monitor, Settings,
Log, Ledger, Works, Capture, Calculator, Notes. Product copy may carry wit;
names, status, authority and failure messages do not.

## 3. The works: how the system is put together

### 3.1 Layers

```text
kernel (Rust)                          handles, ring, memory, devices, filesystems
  -> user runtime shim (audited)       the platform seam: rings, entry, volatile access
  -> base components (Wizzy)           init, works, plumber, history, log, clip, settings, ledger
  -> the bench (Wizzy)                 compositor, rail, strip, palette, deck, greeter
  -> apps (Wizzy), annex (foreign)     Terminal, Files, Editor ... ; ports
```

The runtime shim is the only non-Wizzy code above the kernel, under the second
safety boundary `015` describes. Everything from init upward is Wizzy source
whose behaviour is part of this OS (D-183).

### 3.2 A component and its manifest

A component is one Wizzy program plus a manifest. The manifest is a document in
the settings language (section 12) with these fields:

| Field | Owner | Meaning |
|---|---|---|
| `component` | author | The stable identifier, `works.history` or `app.files` |
| `artifact` | build | The D-226 envelope identity: format, target, platform ABI, artifact digest and provenance |
| `requires` | compiler | The derived effect set, copied from the envelope's `effects.required`; a request, never a grant (`015`, D-183) |
| `exposes` | author | Typed interfaces this component serves, each versioned: `history.v1`, `plumb.v1` |
| `depends` | author | Other components with a readiness kind: `after` (ordering only), `needs-ready` (do not start until it answers), `waits-for` (start together, tolerate absence) |
| `start` | author | `lazy` (on first message), `eager` (at boot), `manual` |
| `supervise` | author | On failure: `restart` with a bounded rate, `stop`, `propagate` to the parent, or `convert` to a report |
| `quota` | author, checked against the parent | Frames, handles and child slots this component may hold |
| `agent` | author | Present when the component acts on the user's behalf without a user in the loop (section 4.6) |

Fields the compiler owns cannot be edited by hand: the image build regenerates
them from the artifact, and a mismatch refuses the image.

### 3.3 The base graph is static and verified

The base system's components, their dependency edges and their routes are
compiled at image build into one document, the works file, and verified there:
the graph is acyclic; every `depends` names a component in the image; every
route names a provider that `exposes` the interface at that version; every
quota fits inside its parent's; every `requires` is covered by a grant in the
grants document or is explicitly unmet, in which case the component is marked
degraded rather than silently broken. The boot self-test reads the works file
back and compares it with what init actually built.

The lesson is Hubris's and s6-rc's: a base system whose set of components is
fixed cannot fork-bomb itself, and a graph verified offline fails at build time
rather than at 3 a.m. User-installed apps are dynamic leaves under the bench;
they cannot become parents of base components.

### 3.4 init, supervision and lazy start

init is the root of the works. It holds the first bootstrap channel, builds the
works supervisor from the works file, and then does nothing else. The
supervisor owns every other component's lifetime through the process objects
the kernel will expose (`015` slice 7): it starts, observes exit status,
applies the `supervise` policy, and reports every failure to the log with the
component's incarnation number. A panic in one component ends that component
(`002` pillar 6, and Wizzy's fault-domain rule); the supervisor decides what
happens next.

Nothing runs because it might be needed. A `lazy` component's exposed
interfaces have front doors: channels the supervisor holds. The first message
to a front door starts the component and forwards the message; a component that
declared `retire-when-idle` is stopped after its quiet period, and its state
lives in places, not in memory, so stopping it loses nothing. This is
launchd's on-demand start generalised from sockets to channels, and it is how
`002` pillar 4's "no daemons by default" is kept true rather than aspired to.

### 3.5 Parents pay for children

Quotas flow down the tree. A component's `quota` is charged against its parent's
remaining quota when it starts; a child that asks for more than its parent has
left is refused with a typed error before it exists. The bench is the parent of
every app, so an app's cost is visible in one place. The kernel side of this
is PW-700, which the register holds as a trigger for exactly this caller.

### 3.6 The works as the admin view

The Works app draws the live graph: components as nodes, channels as edges.
Selecting a node shows its holdings, its trusted base (everything it depends on,
transitively, which is what it must trust), its quota and usage, its incarnation
and the tail of its log. Editing a route or a policy edits the works file, and
the supervisor applies the change when the document is written, the way
Sculpt's deploy file does. There is no second configuration path.

### 3.7 The log

Every record has the same shape: a monotonic timestamp (wall time is added
when a clock exists, section 15), a component id and incarnation inserted by
the supervisor rather than the program, a span id inserted by the runtime, a
level, a message and typed fields. The split between fields the program wrote
and fields the supervisor asserted is what makes filtering trustworthy
(journald's lesson). A component may also publish a live state document, an
inspect document, that the Works app and the shell read directly. The Log app,
the shell and the Editor read one schema; a log line that names a file and
line is plumbable (section 5.3).

## 4. The ledger: authority as a visible material

### 4.1 What a program starts with

A program is born with one handle, its bootstrap channel (D-111). Its launcher
sends the bootstrap message, and everything in that message is a grant the
launcher chose: places, a terminal, service front doors, a settings capability.
For a base component the launcher is the works supervisor and the grants come
from the works file. For an app the launcher is the bench and the grants come
from the program's grant document.

### 4.2 Two documents, not one

`013` left open where a program's requirements are declared: in the executable
or in a package. The answer is two documents, because they are two things.

- **Requirements** are derived by the compiler from the program's effects
  (`uses`), carried in the D-226 envelope as `effects.required`, and copied into
  the package manifest. They are read-only evidence: nothing the program says
  about itself grants anything (`015`, "Authority and effects").
- **Grants** are the user's decision, kept by the bench in a per-program grant
  document that names the artifact digest the decision was made against.

The ELF `PT_NOTE` reservation `013` made stays empty. D-316 records this and
resolves `013`'s decision 5.

### 4.3 The first launch shows a diff, never a list

When a program is installed, or when a new version arrives whose requirements
differ from the ones the grant document was written against, the bench shows
the difference: what is new, what went away, and which of the new items are
full-authority-equivalent (the language's manifest rule). The user accepts,
narrows or refuses, and the decision is written to the grant document with the
digest. A list of permissions gets rubber-stamped; a delta gets read. In a
non-interactive session the same situation refuses to launch and says why.

### 4.4 No prompts, ever

At run time a program that lacks a holding does not get a prompt on the user's
behalf. It gets a typed refusal, and the user interface built on the toolkit
turns that refusal into the gesture that would grant it: a picker for a place,
a drop target for a file, a plumbing rule for a kind of content. The three
traditions that took capability security seriously in a user interface
arrived at the same rule independently (`018` section 3.4): derive authority
from what the user is already doing, and show the graph, never ask.

The gestures that grant:

- **The picker.** The only file dialog is drawn by the bench and returns a
  capability to what was chosen, attenuated to what the program asked for
  (read, or read and write). The program never sees a path it was not given.
- **The drop.** Dropping a file or a place from Files onto a window grants the
  dropped thing to that window's program.
- **The plumb.** A plumbing message carries the holding it refers to, attenuated
  by the plumber to that one item (section 5.3).
- **The window drop** (experimental, section 14). Dropping one window onto
  another hands over the dragged window's primary holding.

### 4.5 Labels, and the Ledger

The compositor draws a trust label in every window's title bar: one line of
text summarising the program's holdings (which places, whether it can reach
the network, whether it reads the clip) and a band whose colour marks the
trust class, first-party, user-installed or annex. The text always says what
the colour says (colour never carries status alone, per the brand rules), and
the program cannot draw over the label because the compositor owns that
region. Qubes proved that an unforgeable label from outside the compartment is
what lets a user reason at a glance.

The Ledger app lists, per program, its live holdings, its grant history and
its refusals, and offers narrow, extend and revoke. Revoke is honest about its
limits: the kernel has no revocation today (`011` says so and names the
per-object epoch bump that would give it one), so version one revokes by
ending the program and editing its grant document, and the epoch mechanism is
listed in section 15 with the Ledger as its caller.

### 4.6 Agents are programs

A component whose manifest says `agent` acts on the user's behalf without a
user in the loop. It runs in restricted mode (the language's host modes; the OS
has no open mode), its label says `agent`, every command it invokes is logged
under its own identity and appears in the teach-me stream (section 6.3), and it
cannot grant to itself: only a user gesture mints a grant. There is no
always-on capture of the screen or the keyboard; a program that wants to see
the screen holds a capture capability that the label shows. Windows' 2026 agent
security model reached the same place by adding identities, session isolation
and audit after the fact; here they are the ordinary rules for any program.

## 5. Places, files, plumbing, the clip and time

### 5.1 Places instead of a root

There is no `/`. An absolute path is a malformed argument (`014`). The bench's
places service holds the directory capabilities the user has named, and Files
shows them in its sidebar. A place is created by the user through the picker
over the storage the works exposes, by an install (a package's data place), or
by the system (the Settings place, and later the Snapshots place). A place
backed by tmpfs wears a `volatile` badge in every list and title, because
`015` says writes are honestly volatile until WizFS.

An app sees exactly the places it was granted. `pwd` in a shell reports the
shell's logical path from the place it was given; a directory capability has no
parent (`014`), so the logical path is the shell's record of how it got there,
not a kernel fact.

### 5.2 Files

Column view, dual pane, tabs, a Quick Look on Space, sort and filter, batch
rename, and a search box that answers instantly by filename over the granted
places. Instant search needs a change journal from the filesystem (Everything's
model); the journal is a kernel prerequisite in section 15 with Files as its
caller, and until it exists search is an enumeration with a progress bar that
says so. Attributes as columns and queries saved as live documents (Haiku's
model) arrive with WizFS in Phase 13, because they need an index the OS owns.

### 5.3 Plumbing

Programs do not know each other. When a program has something for someone
else, it sends a plumbing message: a source, a type, attributes, data and the
holding the message refers to. The plumber matches the message against a
rules document the user can edit, attenuates the holding to the one item the
message names, and delivers it to the port a rule chose. A click on
`nvme.rs:412` in a log line opens the Editor at that line with a read
capability to that file only; a URL goes to the browser in the annex; an image
goes to Viewer. "Open with" is a plumbing rule. The rules document applies
when written, and the default rules ship with the image. This is Plan 9's
plumber with capabilities attached, and it replaces the intent and URL-handler
mechanisms of the mobile platforms with a document.

### 5.4 The clip

Clipboard history is a base component. Entries are MIME-typed, pinned or
unpinned, kept for seven days or until pinned, and searchable from the
palette. The focused program may read the current entry; any other read
requires a `clip.read` holding, which the label shows, and every such read is
logged. kitty's clipboard protocol reached the same consent rule for terminals.

### 5.5 Time

Uptime is the only clock today (D-152, D-166), so the log and Files show
durations and relative times until a wall clock exists (section 15). When
WizFS arrives, snapshots appear as places named by when they were taken, and
"yesterday" is a place a program can be granted like any other. Rewind, the
combination of snapshot places with the command log, is an experiment in
section 14 and not a promise.

## 6. The command registry and the palette

### 6.1 A command is a record

Every action a program offers is a command record: an id in `noun.verb` form
(`file.rename`, `column.widen`), typed parameters, the effect set the action
needs (derived from the program's manifest), a default binding, a menu
placement, a context predicate that says when it applies, and, where one
exists, its inverse. The toolkit registers a program's command table and
exports it over a typed interface; the bench merges the tables of every
running program with its own.

One registry serves every surface: window menus and context menus are
rendered from it, the palette searches it, the shell calls it, scripts and
agents invoke it, jump lists on the rail are its pinned entries, the
accessibility tree exposes it as actions, and the test harness drives programs
through it rather than through synthesized input. macOS's menu discipline
(every command exists in a menu with its shortcut) is kept; the menus live in
the window, Windows-style, and the palette is the same inventory searched.

### 6.2 The palette

Bench+Space opens the palette as a quasimode: hold the key and it is there,
release and it is gone, or tap to pin it. It is fuzzy, answers on the first
keystroke, and runs a bus of providers: commands of the focused program,
commands of the bench, apps, files in the granted places, the clip, a
calculator, unit and colour conversion, and settings pages. Quick keys are
learned from use: after the user has chosen "Terminal: new column" for `tnc`
twice, `tnc` runs it (Tahoe's Spotlight taught this). Above the result the
palette shows what will happen: the effect set of the command and the holdings
it will use, so a user sees `uses fs.write(Projects/os)` before pressing Enter.
There are no web results.

### 6.3 Teach-me mode

A setting. When it is on, every command invoked through a menu, a shortcut or
the palette is echoed into the Terminal's current notebook as the line that
would have done the same thing, runnable and editable. It is how the shell is
learned without a manual, and it is how an automation is written by doing it
once. AutoCAD's command line did this for decades; nothing on the desktop has.

### 6.4 Type-to-jump

Bench+J, held, then typing, moves keyboard focus to the first visible text that
matches, across every window on the output. The compositor already holds the
accessibility tree of every surface (section 9.6), so the bench can search it
without any program's help. This is Raskin's LEAP made possible by push-model
accessibility. It is an experiment (section 14).

### 6.5 The bench key map

| Keys | Action |
|---|---|
| bench+Space | The palette |
| bench+Left, Right | Focus the previous or next column; with Shift, move the window there at half width |
| bench+Up, Down | Cycle the focused column's stack; with Shift, move the window within the stack |
| bench+Z | The snap popover: column widths and stack placement |
| bench+F | Pop the window out to floating, or back into the strip |
| bench+Enter | Fill the column (the strip's maximise) |
| bench+1 to 9 | The rail's pinned slots |
| bench+Tab | The overview: the strip zoomed out, workspaces and bays |
| Alt+Tab | Most-recently-used windows with live previews |
| bench+PgUp, PgDn | The previous or next workspace on this output |
| bench+E, T, comma | Files, Terminal, Settings |
| bench+V | The clip |
| bench+Shift+S | Capture |
| bench+J | Type-to-jump (held) |
| bench+L | Lock; the lock screen is the greeter's own code path |
| bench+? | The shortcut guide, generated from the registry of the focused program |

Every binding is a command record, so every one is rebindable in Settings and
listed in the guide.

## 7. The shell

### 7.1 What the shell speaks

The shell speaks Wizzy's expression syntax over Wizzy's values: literals,
variables, calls with labelled arguments, pipelines with `|`, and the string
interpolation the language already has. It evaluates a bounded command
sublanguage itself, single-pass and without macro processing (rc's rule): a
line is parsed once, lists are values rather than strings that split, and
there is one quoting rule. Anything larger than a command line is a Wizzy
program compiled where the compiler runs, which is the host until the compiler
runs on the target (`015`, "Deliberately later"). The shell therefore needs no
compiler on the target: commands are compiled artifacts with manifests, and the
shell composes them.

Commands follow a `verb noun` grammar with a queryable registry (section 6):
completion, help and the palette derive from the same signatures. Coreutils
are typed commands: `ls` returns a table of entries, `cat` returns text or
bytes, `pwd` returns the logical path, `echo` returns text.

### 7.2 Typed pipes and the wire format

A pipe carries Wizzy values. Between two stages the values are encoded in the
wire format: a versioned, self-describing, streaming encoding of the
language's serialisable types, identified as `wizard-wire-v1`, defined in a
host-testable crate before the shell is written, with a canonical form so two
encoders agree byte for byte. The format is exterior-first (YSH's rule): it is
the contract, and in-memory representations serve it rather than the other way
round. `bytes` is the explicit escape hatch: a stage that produces or consumes
raw bytes says so in its signature, and the `lines` and `text` adapters convert
at the boundary. A foreign program in the annex always speaks bytes.

The type contract at a boundary is decided before the first record, not
inferred from the first ten rows: a stage declares its output type or declares
it unknown, and a consumer that needs a known type refuses an unknown one with
a message that names the adapter to add. nushell's users reported the other
policy's failure mode often enough (`018` section 4.1) for this to be a rule.

### 7.3 Effect preview

Before a line runs, the shell shows what it may do. The effect set of a
pipeline is the union of its stages' `requires` fields, narrowed to the
holdings the shell will actually pass; no compiler is needed because every
stage is an artifact with a manifest. The preview sits under the prompt:
`uses fs.read(Projects/os), io` for a `ls` over a place, and
`uses fs.write(scratch)` in a different colour and with the word `write` when a
stage will mutate. A stage whose requirements the shell cannot cover shows
the missing holding and the gesture that would supply it.

### 7.4 Jobs, wires and cancellation

A pipeline is a job. A job is a structured-concurrency scope in the shell's
runtime: its stages are children, it ends when they end, and cancelling it
cancels every in-flight kernel operation with the ring's Cancel opcode and then
the scope. Pipelines are explicit channels with no inherited descriptor table
and no `fork` (`015`, verification rule "Shell is non-POSIX-native"), so a
running job can be drawn: the Terminal's wires view shows the stages, the
channels between them and how full each channel is. Backpressure is a thing the
user can see rather than a thing that happens.

### 7.5 History, prompt and defaults

History is a base component, not a file: every line is a record with its exit
status, duration, logical directory, holdings used and session. Ctrl+R opens a
filterable view over those records; the palette searches them too. Prompts emit
their marks natively through the session protocol, so jump-to-previous-command
and select-output are shell features rather than terminal heuristics. Defaults
are fish's: autosuggestion from history, syntax highlighting, no configuration
needed for any of it. Scripts and configuration have a compatibility policy
from the first release, because the structured shells that lacked one lost
their users' trust to breakage.

## 8. The session protocol and the Terminal

### 8.1 Records, not a byte grid

The native terminal is not an emulator of a 1978 device. Between a shell and
whatever displays it runs a session protocol whose unit is a record:

| Record | Carries |
|---|---|
| `Prompt` | The logical directory, the holdings in scope, a prompt mark |
| `Command` | The line as typed, its job id, its effect preview |
| `Output` | A stream of chunks: a text run with attributes, a typed value in the wire format, an image, a hyperlink, a progress mark |
| `Status` | Exit status, duration, bytes moved |
| `Input` | A key event with press, repeat and release, modifiers and layout-independent alternates, or a text commit from an input method |
| `Resize`, `Focus` | Geometry and focus changes, as records rather than signals |

Each command's output has its own grid, so a stage that moves a cursor cannot
corrupt an earlier block (Warp's design), and a typed value in the output is
rendered by an inspector rather than flattened to text. Keyboard input is
defined once, unambiguously, with the kitty protocol's flag set as the
reference feature list; the terminal never guesses whether Ctrl+I was Tab.
Images are placed by id with a z-order and survive scrollback; text sizes for
headings are native; hyperlinks and MIME-typed clipboard entries are native,
with the clip's consent rule for reads.

VT100 is a compatibility component: in the annex, a foreign program's byte
stream is decoded by an emulator into the same records, so a ported editor or
`curses` tool appears inside a block like anything else. The emulator is
isolated behind the protocol and never becomes the native surface. This is the
position both terminal camps in `018` section 4.2 would accept: implement the
old zoo correctly, and do not build on it.

### 8.2 The Phase 7 terminal

Before the bench exists the session protocol's first renderer is the Phase 7
console: a line renderer over the console capability that draws text runs,
values as tables and prompts as prompts. That renderer is disposable; the
records are not. `015` slice 8's TTY protocol is this protocol.

### 8.3 The Terminal app

A block notebook. Each command is a block with its prompt, line, output and
status; blocks fold, are searchable, and can be re-run or edited in place.
Typed values in a block get inspectors (a table with sortable columns, a tree
for nested records, an image, a diff) in the manner of moldable tools, and a
value can be dragged out of a block as a holding. The wires view (section 7.4)
is a block's alternate rendering. Multiplexing and remoting belong to this
layer: a session is a thing that can be detached, reattached and, when the
network exists, served to another machine over the same records.

## 9. The bench

### 9.1 The strip with snap semantics

Each output is an infinite horizontal strip of columns. A column holds a stack
of windows shown as tabs; one window in the stack is visible. Column widths
snap to a third, a half, two thirds and full, the fractions Windows users
already have in their hands; bench+Z offers them in a popover, and bench+Left
or Right with Shift moves a window into the neighbouring column at half width.
A new window opens at its natural width beside the focused column and never
resizes the column the user is reading (niri's rule, the reason the strip
exists). bench+Enter fills the column. Scrolling the strip is a gesture on the
pointer and bench+Left or Right on the keyboard; the visible part of the strip
is always whole columns.

A bay is a named segment of the strip, saved with its windows' identities,
widths and stack order, and recalled from the rail: Windows' snap groups with a
name. Bays are also documents (section 14), because a workspace worth naming
is worth reopening tomorrow.

Floating is one key away. bench+F pops a window out above the strip on the
same workspace; a floating window is placed the way GNOME's Mosaic proposal
describes, centred at its natural size with neighbours sliding aside, and it
snaps to the same fractions when dragged to an edge. Tool palettes, pickers
and small utilities float by default; everything else starts in the strip.
Windows-style floating-with-snap is therefore a mode of the same shell rather
than a second shell, and a user who wants it everywhere sets floating as the
default for new windows.

Workspaces stack vertically per output (bench+PgUp and PgDn); they are dynamic,
created when used and removed when empty, and can be named. There is exactly
one organising hierarchy: outputs hold workspaces, workspaces hold a strip,
strips hold columns, columns hold stacks. Activities, spaces and stage-like
groupings are not added beside it.

### 9.2 The rail

The rail is at the bottom by default and moves to any edge. Its buttons are
labelled and ungrouped by default, one per window, with the window's trust
band; pinned entries launch or focus; a right-click shows the program's jump
list, which is its pinned commands from the registry; bench+1 to 9 select the
pinned slots. The rail stays visible when a window fills its column; nothing
takes the screen without asking, and a program that asks (a game, a video) is
labelled fullscreen and gets automatic do-not-disturb with a summary of what
was missed when it ends.

Background programs do not draw in the rail. A component that wants a presence
declares it: a status string, a badge count, and a menu of commands from its
registry, rendered by the bench in a strict area. There are no tray pixmaps and
no injected menus, which is the diagnosis GNOME made without shipping the
replacement.

The rail also carries the clock, quick settings (network, sound, display,
appearance, contrast, motion) and notifications. Notifications stack per
program, keep their actions in history, and never carry advertisements,
recommendations or tips.

### 9.3 Chrome, focus and click-through

The compositor draws every window's chrome: a 32 px title bar with a 16 px
program icon inset 16 px, the title, the trust label, and caption buttons with
rest, hover, pressed, active and inactive states; every non-interactive pixel of
the bar drags; a right-click shows the system menu; a double-click fills the
column. Corners are 0 px in the strip and 8 px when floating, the Fluent rule
that keeps tiled edges gapless. Because the compositor owns the chrome, a hung
window is still closable from its frame, the argument against client-side
decoration that no toolkit answered.

Focus is click-through: a click on a background window acts on it as well as
focusing it, the behaviour switchers miss most on macOS. Maximise means the
column, and the rail stays.

### 9.4 Session restore, the greeter and safe mode

Session restore is a compositor primitive from the first release, not a portal
bolted on later: every toplevel has a restore token, the bench records each
window's program, token, column, width and stack position, and at login the
bench relaunches programs by identity and hands each its token. Bays are the
same mechanism with a name.

The greeter is the bench's own code path with a different policy: the lock
screen and the login screen run the same compositor, the same input method,
the same accessibility, the same network and sound controls, so nothing exists
twice. First-run setup is a separate program launched once, not part of the
installer. A safe mode boots the base graph with every user-installed
extension, theme and autostart disabled and says so on the first screen.

### 9.5 The compositor protocol

The compositor speaks its own protocol over channels and shared memory
objects (D-11, `002`), 2D rectangles only, with a surface tree, damage,
presentation timing, fractional scaling per output and colour management from
the start. Privileged parts of the protocol (layer surfaces for the rail and
the palette, foreign-toplevel listing for the overview and Alt+Tab, capture
for Capture, the clip) are separate interfaces granted by capability to bench
components, never available to an app by asking. A Wayland-shaped model with
the privileged surface split out by capability is the security model Linux
converged on, made explicit.

### 9.6 Input, accessibility and text input

The toolkit produces an accessibility tree for every widget as a by-product
of the widget tree, and pushes it to the compositor, which holds the tree of
every surface. The screen reader, the magnifier, the automation harness,
type-to-jump and agents all read that one tree; no program can exist without a
role. A screen reader and a magnifier ship as OS components in the first
graphical release, not as apps found later.

There is one input-method owner: the compositor. A program receives preedit
ranges and commits through the session or toolkit protocol and reports its
cursor rectangle; the compositor draws candidate windows at that anchor.
Layouts are a bench setting applied to every surface, and layout-independent
key alternates are part of every key event.

Keyboard first means every action is a command with a binding, focus is always
visible and never obscured, and target sizes and focus appearance follow WCAG
2.2 in the toolkit rather than in each app. A pointer works everywhere and is
required nowhere.

## 10. The apps

All first-party, all Wizzy, listed with the phase in which each can first
exist given the roadmap in `000-vision.md`.

| App or component | Purpose | Phase | Shape |
|---|---|---|---|
| init, the works supervisor | The root and the tree | 7 | Section 3 |
| shell, coreutils | The typed shell and `echo`, `cat`, `ls`, `pwd`, then `mkdir`, `cp`, `mv`, `rm` in tmpfs | 7 | Section 7; `015` slices 8 and 9 |
| history, plumber, log | Base components | 7 | Sections 5.3, 7.5, 3.7 |
| palette (text mode) | The registry searched from the console | 7 | Section 6.2 without the compositor |
| network service and a client | The first service over the Phase 8 stack | 8 | Wizzy-first per D-183 and `000` |
| the annex | Ports and their POSIX root | 9 | Section 13 |
| compositor, toolkit, rail, strip, deck, palette, greeter | The bench | 10 | Section 9 |
| Terminal | The block notebook | 10 | Section 8.3 |
| Files | Places, columns, dual pane, Quick Look, search | 10 | Section 5.2 |
| Editor | Selection-first modal editing, tree-sitter syntax, LSP, DAP and ACP native, a deferred plugin surface, a CRDT buffer, the OS text stack | 10 | Helix's shape, not VS Code's |
| Settings | Every page generated from schema, every page addressable, no legacy panel | 10 | Section 12 |
| Ledger, Works, Log | The three system views | 10 | Sections 4.5, 3.6, 3.7 |
| Viewer | Images, documents, values | 10 | Plumbing's default sink for content |
| Monitor | Processes, channels, quotas, frames, rings | 10 | Sysinternals depth, one tree |
| Capture | Region, window, scrolling and video capture as a pipeline: capture, annotate, plumb | 10 | ShareX's shape; OCR when a model exists |
| Calculator, Notes | Small, quiet, instant | 10 | Also palette providers |
| screen reader, magnifier | OS components | 10 | Section 9.6 |
| NetSurf | The document web, in the annex | 11 | D-75 |
| audio service, USB input | Sound and a mouse | 12 | `000` Phase 12 |
| queries, snapshots, packages, rewind | Everything that needs a writable, indexed, snapshotting filesystem | 13 | D-73 |

No first-party app carries an assistant button. An assistant, if one is
installed, is an agent under section 4.6.

## 11. Packaging and updates

### 11.1 Artifacts and packages

A program ships as a reproducible artifact in the D-226 envelope: format,
target, platform ABI, versions, effect requirements, provenance and digest. A
package is an immutable, content-addressed set of components with a signed
provenance record and the derived manifests of section 3.2. The store is
addressed by digest; two installs of the same bytes are one entry.

Until WizFS exists the image is read-only and the store ships inside it, which
is what `015` means by host-built, checksum-pinned programs. Installation,
which is profile mutation, arrives in Phase 13 with the writable filesystem: a
profile is a document naming the components a user has and their grant
documents, and every change produces a new generation that the previous one
can be rolled back to from the boot menu or the Updates panel.

### 11.2 The image

The system image is A/B: an update is written to the inactive slot, verified,
and booted once; a boot that does not reach the bench falls back. Generations
of the base image and of the profile are the two axes of one Updates panel that
shows current, previous and rollback and nothing else. Safe mode (section 9.4)
is a boot of the current image with the user's layers disabled. Reproducibility
is a property of the package format: the digest in the envelope is what the
image build verifies, and two clean builds must agree (`015`, verification
rules).

## 12. Configuration and settings

One language configures everything: the works file, manifests, grant
documents, plumbing rules, bench layout, app settings and themes. It is a
node-shaped document format in the KDL family (nodes with arguments,
properties and children, comments, a version tag) whose schemas are Wizzy
types, so a document is checked the way a value is checked and an unknown
field is an error, not a warning. There are no silent overrides: where two
documents contribute to one setting the merge rule is declared in the schema,
and a value that is concrete is final, in CUE's sense. The concrete syntax is
fixed with the settings service (section 18) and is the same on the host, in
the image build and on the target.

Every setting has an id (`bench.rail.edge`, `terminal.font.size`) and a schema,
and every settings page is generated from the schema, so a setting without a
page cannot exist and a page without a schema cannot be drawn. Every page is
addressable (`settings:bench/rail`) from the palette and from plumbing. Settings
documents live in the Settings place; they are diffable, and with WizFS they are
snapshotted, so "what changed since yesterday" is a question with an answer.
There is no second settings application and no legacy panel to migrate.

## 13. The annex

Foreign programs, the mlibc ports, CPython, `links` and NetSurf of Phases 9 and
11, run in the annex: a compatibility container whose POSIX root is
synthesized from the places the user granted it, whose terminal is the VT
compatibility component of section 8.1, and whose windows wear the annex trust
band. Inside the annex a program sees the conventions it expects, a root, a
current directory, descriptors, signals as the gateway defines them; outside
it holds exactly the capabilities it was given, because the kernel does not
know what POSIX is. The label says "annex" so nobody mistakes the inside for
the outside. This is `010` section 5's conclusion applied: conform at the
boundary, and keep the boundary visible.

## 14. Experiments

Each experiment ships off by default where it would change muscle memory, is
measured, and is removed after one release if it fails its criterion. Arc's
Spaces reached 5.52% of daily users and the company retreated; an experiment
that is not measured is a novelty tax waiting to be paid.

| Experiment | Criterion to keep |
|---|---|
| Effect preview under the prompt (7.3) | On by default; kept unless it is turned off by the user within a month, since it costs nothing |
| Type-to-jump (6.4) | Used in more than a fifth of sessions after three months |
| The wires view (7.4) | Opened for more than a tenth of multi-stage jobs |
| Teach-me mode (6.3) | Produces at least one saved automation per month of use |
| The window drop (4.4) | Used at all, and never the cause of an unintended grant in the ledger |
| The deck (2) | The user does not recreate desktop icons elsewhere |
| Bays as documents (9.1) | More than one bay reopened per week |
| Rewind (5.5, Phase 13) | A rewind is performed and kept at least once a month |

## 15. The two ledgers

### 15.1 Kernel prerequisites, with their callers

These are post-gate items in the sense of `016`'s "Explicitly after the
compiler gate": each is a language-neutral platform prerequisite named with the
Wizzy caller that D-183 requires, and none enters the register by being
relabelled foundation work.

| Prerequisite | First caller | Where it already stands |
|---|---|---|
| Channel transport: send, receive, pair creation, close | init and the works | `015` slices 3 and 8 |
| The versioned bootstrap message | Every launcher | `015` slice 3 |
| Console and input `Device` capabilities | The Phase 7 terminal | `015` slice 4, D-172 |
| Directory enumeration from ring 3 | `ls`, Files | `015` slice 8 |
| Memory objects with map and unmap; runtime allocation; stack growth | The runtime, then the compositor's buffers | `015` slice 5, PW-701 |
| Process objects, spawn from an executable handle, exit status | The works supervisor | `015` slice 7 |
| A multi-program image and loader selection among images | init | `015` slice 7 |
| Quotas: frames, handles, children | Parents paying for children (3.5) | PW-700 |
| A display object and a compositor-owned framebuffer or GPU surface | The compositor | `000` Phase 10 |
| Input events delivered over channels, including a mouse | The compositor | `000` Phases 10 and 12 |
| Handle revocation by epoch | The Ledger's revoke (4.5) | `011`'s deferred mechanism |
| A filesystem change journal | Files search (5.2) | New; Phase 13 candidate |
| An attribute index and live queries | Files queries | Phase 13, D-73 |
| An RTC wall clock | The log and Files timestamps | `000` Phase 12 |
| Randomness as a facility or a capability | Session and job ids | `013`'s open question, Phase 8 |
| Network sockets | The Phase 8 service | `000` Phase 8 |

### 15.2 Wizzy assumptions, mapped to the language roadmap

Rows name the wizards-ink ledger identifiers as they stood on 2026-09-12; the
language repository owns their meaning and schedule.

| Assumed | Row | Used by |
|---|---|---|
| The platform seam: one target driver for handles, ring I/O, monotonic time, console, spawn and the bootstrap channel, `no_std` plus `alloc` | G-082 | Everything |
| Attenuation from the root capability and capability-taking primitives | G-070 | The ledger, the picker, plumbing |
| `Bytes` as a first-class value | G-049 | The wire format, the session protocol, files |
| Explicit interfaces | G-046 | `exposes`, the command registry |
| Effect rows on stored functions | G-047 | Command records that store callbacks |
| `Transfer` of resources between tasks | G-051 | Jobs handing holdings between stages |
| `supervise` | G-090 | The works supervisor |
| Reproducible packages, install-time diffs, sandboxed build plugins, provenance | G-073 to G-076 | Section 11 |
| The two foreign boundaries | G-077, G-078 | Text shaping and locale data bindings for the toolkit, Wasm modules under D-72 |
| `std.secrets` | G-134 | Session tokens when the network exists |
| Host semantics with no assumed root, environment or wall clock | G-081 | Everything in section 5 |
| Restricted mode only, no open mode on this OS | G-071 | Agents, the annex boundary |
| Actors and multicore | M6 | Services at scale; not needed by the first release |

Needs this design raises that the language roadmap did not name on that date:
a reactive user-interface story (view functions diffed into a retained widget
tree, with the purity rules the toolkit relies on enforced by the language);
a canonical, streaming, versioned wire format for the language's serialisable
types; measured density for four or more concurrent processes on the chosen
route (`015`'s threshold); and green threads that park on completions rather
than block the ring. Each is a question for the language repository, recorded
here so that it is asked once.

## 16. The build order, assuming a complete Wizzy

The order extends `015`'s Phase 7 chain, which it does not change, and then
runs through the graphical and storage phases. Every slice leaves a bootable
system with a check that would go red if the slice were undone.

| Slice | Deliverable | Proof |
|---|---|---|
| 7a | The runtime shim and a Wizzy hello, on the measured route | `015` slices 1, 2, 4 and 6 |
| 7b | Bootstrap channels and handle close | `015` slice 3 |
| 7c | init and the works supervisor v0 over a static graph; the works file read back by the boot check | A component that fails is restarted under its policy; a graph with a cycle refuses to build |
| 7d | The session protocol, the console renderer, the line editor, the typed shell, `echo`, `cat`, `ls`, `pwd`, pipelines as explicit channels | `015` slice 8's proof, plus a typed value crossing a pipe in the wire format |
| 7e | tmpfs mutation and the mutating commands, with the volatile badge | `015` slice 9 |
| 7f | The history component and the plumber v0 | A `file:line` plumb reaches its port with an attenuated read holding |
| 7g | The text-mode palette and effect preview | A pipeline's preview equals the union of its stages' requirements narrowed by the holdings passed |
| 7h | Core Wasm and `wasip1` | `015` slice 10, D-72 |
| 8 | The network service and its client | `000` Phase 8 |
| 9 | The annex with its first port | A port reads a synthesized root built from one granted place and nothing else |
| 10a | The compositor with labels, session restore tokens and the privileged interfaces gated by capability | A window's label cannot be drawn over; a restore token relaunches a program by identity |
| 10b | The toolkit, tokens and the accessibility tree | A widget without a role cannot be built; a theme document changes every app |
| 10c | The rail, the strip, the deck, the palette, the greeter | The bench key map above, end to end, with a pointer disconnected |
| 10d | Terminal, Files, Settings, Ledger, Works, Log | Each app's command registry drives it under the test harness |
| 10e | Editor, Viewer, Monitor, Capture, Calculator, Notes, the screen reader and magnifier | Each reachable and operable without a pointer |
| 11 | NetSurf in the annex | D-75 |
| 12 | Audio, USB input | `000` Phase 12 |
| 13 | Snapshot places, queries, packages and installation, generations, rewind | D-73's definition of done, plus the experiments in section 14 |

## 17. How it will be checked

Protocol, encoding and policy arithmetic belongs in host-testable crates and
runs under `cargo test`; machine behaviour belongs in the boot self-test and
runs under `cargo xtask test-boot`; claims about the tree belong in the
repository checks. Every new check is broken once and watched red, and every
machine check gets a mutation (D-28, `015`). The table names the consequence
of each principle that can go red.

| Claim | Evidence |
|---|---|
| One handle at birth, and every later holding traceable to a gesture or a grant document | The ledger's grant log accounts for every handle a program holds; a handle with no entry fails the boot check |
| No prompts | The bench has no prompt widget; a refusal renders an affordance and the test harness proves the affordance grants |
| Requirements never grant | An artifact whose `requires` names an effect no grant covers is refused at launch, and its manifest cannot be hand-edited past the image build |
| One command registry | Every menu item, palette entry and shortcut resolves to a registered command id; the test harness drives each app only through ids |
| Typed inside | A pipeline's stages exchange only wire-format records; a raw byte stream between two first-party stages is a type error |
| The wire format is canonical | Two encoders produce identical bytes for one value; a decoder refuses a non-canonical encoding |
| Effect preview is exact | The preview equals the union of stage requirements narrowed by holdings passed, for every pipeline in the fixture set |
| The strip never resizes what is read | Opening a window changes no existing column's width; the compositor test asserts geometry before and after |
| The rail is complete on day one | Labels, ungrouping, any edge, drop targets, jump lists and visibility when a column is filled, each a machine check |
| Labels are unforgeable | A program that draws into the label region sees its pixels clipped; the compositor owns the region |
| One toolkit, tokens as API | A theme document change repaints every first-party app; no app carries a private colour |
| Three independent preferences | Contrast, appearance and motion each change alone without affecting the others |
| The works are static and verified | A works file with a cycle, a missing provider or an over-quota child refuses to build; the boot check reads the file back |
| Lazy start | A `lazy` component has no process until its first message, and its first message is delivered |
| Parents pay | An over-quota child is refused before it exists and leaks nothing |
| Volatile is visible | Every tmpfs-backed place and window carries the badge; the badge disappears only with a durable backend |
| The annex is bounded | A port's synthesized root is built from granted places only; an absolute path outside them is not resolvable |
| Session restore | A relaunch after a simulated login restores every window's column, width and stack position from tokens |
| Keyboard-only | Every command in every first-party app is reachable with the pointer device absent |

## 18. Known holes, carried deliberately

- **Revocation.** Version one revokes by ending the program. The epoch
  mechanism is a prerequisite, not a promise (4.5).
- **No wall clock, mouse or sound** until their phases; the design names them
  rather than assuming them (15.1).
- **The settings language's concrete syntax** is fixed with the settings
  service, not here; the design fixes its shape and rules (12).
- **Queries, attributes and snapshots** wait for WizFS; Files ships without
  them and says so (5.2, 5.5).
- **The compiler on the target** is deliberately later (`015`); the shell is
  designed not to need it (7.1), and larger scripts compile on the host until
  then.
- **The GPU** is Phase 10's own question; the toolkit's renderer has a CPU
  backend so the bench runs before it (`020-style-guide.md` fixes the renderer
  rules).
- **Text shaping and locale data** are third-party libraries behind the
  language's foreign boundaries until Wizzy has its own (15.2).
- **The experiments** in section 14 are experiments.

## 19. Decisions this design makes, with their tensions

Each is recorded in `004-decision-log.md`; the tension is why the answer is not
obvious.

1. **Design now, as if Wizzy were complete, while the gate is closed** (D-314).
   The tension: `016` exists to stop kernel polish from becoming userland
   design before the compiler artifact exists, and D-184 says so in its first
   paragraph. The answer is that a design document is the "specification" work
   D-183 admits, it adds no code and no register rows, and its ledgers are the
   handoff the compiler repository asked for; the alternative, designing each
   piece when its phase arrives, is how every desktop in `018` grew two of
   everything.
2. **One command registry** (D-315). The tension: a registry is a tax on every
   app and a constraint on toolkit design. The answer is that every platform
   that skipped it is now retrofitting palettes, automation and accessibility
   separately, and a first-party toolkit can pay the tax once.
3. **Designation is authorization, and requirements and grants are two
   documents** (D-316). The tension: prompts are familiar, and one document is
   simpler than two. The answer is the convergence in `018` section 3.4, and
   the fact that a program's claims about itself must never be its grant.
4. **The shell is typed and effect-previewed, and the terminal is a record
   protocol** (D-317). The tension: byte pipes and VT100 are what every tool
   and every user knows. The answer is that they live on in the annex, that
   the successful terminals are already halfway to records, and that a shell
   whose commands are manifest-bearing artifacts can preview effects without a
   compiler.
5. **The strip with snap semantics, and a complete rail** (D-318). The tension:
   floating windows are the Windows convention, and a strip is unfamiliar. The
   answer is the fusion: snap fractions and the rail are kept whole, floating
   is one key away and can be the default, and the strip's one rule, never
   resize what is being read, is what makes keyboard-first tiling livable.
6. **The works: a static, verified, supervised, lazy graph that is also the
   admin view** (D-319). The tension: a dynamic service manager is more
   flexible. The answer is that flexibility in the base system is how daemons
   accrete, and the user-installed leaves stay dynamic.
7. **Hash-addressed, reproducible artifacts with generations, A/B images and a
   safe mode** (D-320). The tension: a mutable package database is simpler and
   installation is Phase 13 anyway. The answer is that the envelope already
   carries the digest, that generations are the only rollback story users
   understand, and that deciding it now keeps the image build honest.

The style guide, `020-style-guide.md`, records the eighth decision: one toolkit
and one style source, with tokens as the OS API.
