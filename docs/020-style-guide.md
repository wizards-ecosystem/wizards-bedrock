# 020 - The style guide: one toolkit, one style source

How The Wizard's OS looks, moves and reads, fixed as tokens before the toolkit
that draws them exists.

**Standing.** This is a specification alongside `019-userland.md`. The
compiler-readiness gate in `015-wizzy-userland.md` is **CLOSED** under D-183 and
the [pre-Wizzy foundation register](016-pre-wizzy-foundation.md) is the only
pre-gate backlog under D-184; the toolkit, the bench and the apps this guide
governs are Phase 10 work and start after the gate opens. The guide is written
now so that the first widget is drawn from tokens rather than from taste, and
so that `.github/BRAND.md`, which already fixes the ecosystem's palette,
typefaces and voice, is applied to an operating system once rather than
rediscovered per app (D-321).

References: `.github/BRAND.md` and its `brand/tokens.json` version 5;
`018-research-notes-5.md` sections 2.1 (the Fluent 2 numbers), 2.7 (Liquid
Glass), 5.2 (typography and text rendering), 5.3 (tokens, colour spaces,
contrast themes, icon licences) and 7 (accessibility); `019-userland.md`
sections 2, 6.5 and 9. Outside the project: the Design Tokens Community Group
format, OKLCH, WCAG 2.2, libadwaita's accent derivation, Windows contrast
themes, base24, the Open Font License.

---

## 1. Principles

1. **Interfaces are capable instruments, not branded posters.** The brand's
   own rule. Artwork stays outside dense controls; the interface is calm,
   dense enough for a power user, and never decorated for its own sake.
2. **One style source.** Every first-party surface, from the greeter to a
   tooltip in Calculator, is drawn by one toolkit from one token document. No
   app carries a private colour, radius, duration or font size.
3. **Tokens are an OS API.** Third-party programs read the same tokens; a theme
   is a document that changes every program at once.
4. **Three independent preferences.** Appearance (light or dark), contrast
   (standard or high, with an editable palette) and motion (full or reduced)
   are set separately and never inferred from one another.
5. **Keyboard first, focus always visible.** Every control is reachable and
   operable without a pointer, and focus is never obscured or encoded by colour
   alone.
6. **Honest status.** Colour never carries status alone; a state the user must
   not misread gets a colour and a word.
7. **Reduced motion and reduced transparency are hard switches**, not hints.

## 2. The token document

Tokens live in one document per theme in the Design Tokens format: JSON with
typed values, groups, references and composite types, so design tools export
to it directly and the settings language of `019` section 12 embeds it. Colour
values are stored in OKLCH, converted once from the brand's hex values by the
token build, which also records the hex form for reference. Tokens have three
tiers, resolved in order:

| Tier | Example | Who sets it |
|---|---|---|
| global | `color.slate.60`, `space.4`, `duration.fast` | The brand and this guide |
| semantic | `color.canvas`, `color.ink`, `color.accent`, `radius.window`, `motion.enter` | A theme document; this is the tier a theme may override |
| component | `button.background.hover`, `rail.height` | The toolkit, from semantic tokens; not overridable by a theme |

A theme may set semantic tokens only. The toolkit validates a theme before
applying it: every text token against its surface must reach 4.5:1, every
control edge 3:1, and a theme that fails is refused with the failing pair
named. Themes ship as documents in the Settings place; sharing a theme is
sharing a file.

## 3. Colour

### 3.1 Roles

The brand's warm neutral foundation, with the OS's slate-blue accent. Hex
values are the brand's; the token document carries their OKLCH forms.

| Role | Light | Dark | Used for |
|---|---|---|---|
| canvas | `#F7F6F2` | `#232522` | The strip's background, window backgrounds at rest |
| surface | `#FFFFFF` | `#2D302C` | Panes, lists, editors |
| surface muted | `#ECE8DF` | `#383B36` | Sidebars, the rail, inactive tabs |
| ink | `#272522` | `#F7F6F2` | Text |
| ink muted | `#625F59` | `#C9C4BA` | Secondary text, captions |
| line | `#D8D3C9` | `#4A4D47` | Rules, borders, the stroke that replaces a key shadow |
| focus | `#285E63` | `#E3C477` | The focus ring, and nothing else |
| accent | `#485F79` | `#A9C0D8` | Selection, primary actions, the first-party trust band |
| success | `#35633B` | `#AFC9AE` | Completed, healthy, durable |
| warning | `#775A0E` | `#E3C477` | Volatile, degraded, the annex trust band |
| danger | `#8D2F3E` | `#E6A0AA` | Refusal, failure, destructive actions |
| info | `#485F79` | `#A9C0D8` | Neutral notices; the same hue as the accent by design |

The light values meet 4.5:1 against the light canvas and the dark companions do
the same against the dark canvas, which the brand guarantees and the token
build re-checks. The accent used as a foreground (a link, a selected label) is
derived from the accent background by a lightness clamp in OKLab, libadwaita's
rule, so a user-chosen accent stays readable without a second hand-picked
value. The user may choose an accent; the token build refuses one that cannot
be clamped into contrast.

### 3.2 Trust bands and status

The compositor's trust label (`019` section 4.5) uses three bands: accent for
first-party components, line grey for user-installed programs, warning ochre
for the annex. The label's text says the class in words; the band is a second
channel, never the only one. Status colours follow the same rule everywhere: a
badge is a colour and a word, a progress state is a colour and a number.

### 3.3 The contrast theme

High contrast is not dark mode. It is a separate palette at 7:1 or better with
eight roles, Windows' proven set: window, window text, link, disabled text,
highlight, highlight text, control face, control text. It ships as a theme
document the user can edit, and every component maps its semantic tokens onto
those eight roles rather than keeping any colour of its own. Transient surfaces
get a 2 px border in the contrast theme because they cannot rely on shadow.

### 3.4 The terminal palette

The terminal's sixteen ANSI colours and eight bright variants are a base24
slice of the same token document, derived from the brand palette by default so
that a themed terminal and a themed window agree. A user theme that sets the
slice sets it for every terminal and every syntax highlighter at once.

### 3.5 High dynamic range

User interface surfaces stay at reference white; only media surfaces (Viewer,
a video block in Terminal) may exceed it, with the headroom the display
reports. A bright photograph never makes a tooltip dim.

## 4. Type

### 4.1 Faces

| Role | Face | Licence | Notes |
|---|---|---|---|
| interface | Instrument Sans (variable) | OFL | The brand's interface face; weight and optical size axes |
| reading and display | Newsreader (variable) | OFL | Long-form text in Viewer and Notes, section titles; never a substitute for drawn logo lettering |
| code and terminal | JetBrains Mono | OFL | Distinct `0` and `O`, `1`, `l` and `I`; ligatures off by default; texture-healing contextual alternates are a font-build option to be validated by rendering tests |
| accessibility | Atkinson Hyperlegible Next | OFL | Selectable in Settings as the interface face for low vision |

All four are OFL, which permits bundling in an OS and requires shipping the
licence text and renaming any modified face with a reserved name. No face with
another licence ships in the image. Fallback for scripts the four do not cover
comes from a Plex family added under the same licence when the locale needs
it.

### 4.2 The ramp

Sizes are in effective pixels at 100% scale, with line heights, following the
Windows ramp that a decade of use validated. Sentence case throughout.

| Style | Size / line | Weight |
|---|---|---|
| caption | 12 / 16 | regular |
| body | 14 / 20 | regular |
| body strong | 14 / 20 | semibold |
| body large | 18 / 24 | regular |
| subtitle | 20 / 28 | semibold |
| title | 28 / 36 | semibold |
| title large | 40 / 52 | semibold |
| display | 68 / 92 | semibold |

The minimum is 12 px regular or 14 px semibold. There is no bold and no italic
in interface chrome: semibold is emphasis, and italics are excluded from chrome
on readability grounds. Optical size is automatic from the requested size.
Numbers in tables and in the terminal are tabular.

### 4.3 Rendering

One text stack serves the terminal, the editor, the toolkit and documents:
a HarfBuzz-compatible shaper, bidirectional and line-breaking algorithms from
the Unicode data, and a layout engine validated against the web platform
tests, reached through the language's foreign boundaries until Wizzy has its
own. Glyphs are rendered by the same vector rasteriser as every other shape,
through a glyph cache at small sizes. Blending happens in linear light with
gamma correction at output; hinting is light; antialiasing is greyscale at the
high densities every current display system assumes. Subpixel antialiasing is
not used because it fails under animation, scaling and rotation.

## 5. Space, shape and elevation

- **Grid.** 4 px. Spacing tokens: 0, 2, 4, 6, 8, 10, 12, 16, 20, 24, 28, 32,
  36, 40, 48, 52, 56. The 2, 6 and 10 exist to align icon padding.
- **Radius.** Three tokens and no others: `radius.window` 8 px for floating
  windows, flyouts and dialogs; `radius.control` 4 px for controls, bars and
  tooltips; `radius.tiled` 0 px for a window in the strip and wherever edges
  touch. A radius never depends on a window's content; macOS Tahoe showed what
  that does.
- **Stroke.** A 1 px `line` stroke replaces the key shadow on every raised
  surface, which stays crisp at 100% scale. Fine rules over pills, and
  grouping by rule and whitespace rather than by boxes inside boxes.
- **Elevation.** Six ambient shadow levels (2, 4, 8, 16, 28 and 64 px blur) for
  tooltips, flyouts, dialogs and floating windows in that order; dark theme
  doubles shadow opacity. Nothing in the strip casts a shadow.
- **Density.** Default row height 32 px, compact 28 px; the minimum pointer
  target is 32 by 32 px and the minimum keyboard focus target follows WCAG 2.2.

## 6. Material

The base material is opaque and tinted: canvas and surface are solid colours,
optionally tinted by the wallpaper for the strip's background alone. Translucency
is reserved for transient, light-dismiss surfaces (menus, flyouts, the palette,
the snap popover), the Fluent rule, and is never applied to a window body, a
sidebar or a toolbar. A transparency slider exists in Settings, and
"reduce transparency" is a hard switch that makes every transient surface
opaque. There is no glow, no blur behind text, no paper texture inside
controls; the brand's watercolour and ink live in program icons, the greeter's
artwork and empty states, outside dense controls.

## 7. Motion

Motion is calm and functional, and every duration and curve is a token.

| Token | Value | Used for |
|---|---|---|
| `motion.fade` | 83 ms, linear | Opacity changes |
| `motion.enter` | 167, 250 or 333 ms by size, cubic-bezier(0, 0, 0, 1) | A surface appearing |
| `motion.move` | 167, 250 or 333 ms by distance, cubic-bezier(0.55, 0.55, 0, 1) | A window or column moving in the strip |
| `motion.exit` | 167 ms, cubic-bezier(0, 0, 0, 1), always with a fade | A surface leaving |
| `motion.exit.gentle` | 167 ms, cubic-bezier(1, 0, 1, 1) | A dismissed flyout |

Reduced motion replaces every movement with a crossfade of `motion.fade` and
disables parallax and scale; it reduces rather than removes, so state changes
stay perceivable. The toolkit consults the switch; no app does. Nothing
bounces, pulses or loops to attract attention; attention is a badge with a
word and, if enabled, a sound.

## 8. Iconography

System icons are one outline set on a 24 px grid with a 2 px stroke and a
filled variant for selected or active state: Tabler, under the MIT licence,
which covers the interface's needs without a second set. Sizes are 16, 20, 24,
32 and 48 px, drawn to the pixel grid at 16. An icon never appears without a
label or an accessible name.

Program icons follow the brand's instrument style: a drawn instrument in
translucent watercolour with ink flourishes, supplied at 16, 24, 32, 48 and
64 px with real transparency and no card, plus a compact monochrome mark for
the rail at 16 px. First-party apps use the OS's own instrument family; the
Ecosystem's products keep their approved marks. The armillary instrument is
the OS's mark and is not used as an app icon.

## 9. Chrome and layout

- **Title bar.** 32 px high, 48 px when it carries a search field. A 16 px
  program icon inset 16 px, the title in body style 16 px from the icon, the
  trust label after the title in caption style, caption buttons at the end with
  46 by 32 px hit areas and rest, hover, pressed, active and inactive states.
  Every non-interactive pixel drags; a right-click opens the system menu; a
  double-click fills the column.
- **Corners.** `radius.tiled` in the strip, `radius.window` floating.
- **Focus.** A 2 px ring in the focus colour with a 1 px inner ring in the
  contrasting ink, drawn outside the control so it is never clipped or
  obscured; focus is never shown by colour alone.
- **Layout.** Flexbox and grid with logical axes (start and end, inline and
  block) so right-to-left and vertical scripts fall out of layout; there is no
  cascade and no inheritance of style through the tree, only tokens.
- **Scaling.** Fractional, per output, in the compositor; every measure above
  is in effective pixels.

## 10. Keyboard and focus conventions

The bench's own map is in `019` section 6.5. Inside apps:

- Every command has a binding, shown in menus, the palette and the shortcut
  guide.
- Tab and Shift+Tab move focus in reading order; arrows move within lists,
  trees, tables and toolbars; Enter activates; Space toggles; Escape closes
  the innermost transient surface and nothing else.
- Alt plus a letter opens a menu by its mnemonic, the Windows convention;
  mnemonics are shown when Alt is held.
- Typing in a list, tree or table starts type-ahead selection.
- Modal dialogs are rare and always answerable from the keyboard with their
  default action on Enter and their safe action on Escape.
- A control that opens something says so in its accessible name.

## 11. The component inventory

The toolkit's first release draws exactly these, each with an accessibility
role and a command hook: button, toggle, checkbox, radio, text field, search
field, combo, list, table (sortable, tabular numerals), tree, tabs, split,
toolbar, menu bar, context menu, palette result list, dialog, sheet, toast,
progress, badge, label, link, breadcrumb, notebook block, inspector (table,
tree, diff, image), trust label, empty state, error state. Anything else is
composed from these.

## 12. States and copy

An empty state says what the user can do next and offers the command. An error
state names the refusal in the kernel's or the language's terms, names the
gesture that would fix it, and never blames the user. Status uses the brand's
four words, Working, Frozen, Specified and Gated, where a component's standing
is shown. Copy is sentence case, plain, concrete, and free of hype; humour
stays away from safety, authority, status and failure. Product names appear in
full on first mention, then the project word.

## 13. Sound

Silent by default. A token set of four sounds (attention, done, refusal,
alarm) exists for users who enable it, each paired with a visible cue so sound
is never the only channel. No start-up sound.

## 14. Themes by users

A theme is a token document that sets semantic tokens; a wallpaper pair may be
attached; a theme may declare a day and a night half, and the bench switches at
the user's schedule. The toolkit validates contrast before applying a theme
and refuses one that fails. Sharing a theme is sharing a document; a community
palette is a document that sets the same slots, which is what the theming
culture in `018` section 1.6 wanted and never had from an OS.

## 15. How it will be checked

| Claim | Evidence |
|---|---|
| The token document is complete and consistent | A host test resolves every semantic token, checks every text and edge pair for contrast, and fails on any missing or circular reference |
| No private colour, radius, duration or size | A repository check over first-party sources rejects a literal colour, radius, duration or font size outside the token document |
| A theme changes everything | A machine check applies a theme document and asserts every first-party window repainted from the new tokens |
| Three independent preferences | Changing contrast, appearance or motion alone leaves the other two tokens untouched |
| Chrome numbers | The compositor's tests assert bar height, icon inset, hit areas, radii in the strip and floating, and the focus ring geometry |
| Reduced motion | With the switch on, the test harness observes no animated property other than opacity |
| Keyboard reach | Every component in section 11 is operable in the harness with no pointer device |
| Licences | The image build refuses a font or icon file whose licence is not in the allowed list |

## 16. Known holes, carried deliberately

- OKLCH values are computed by the token build from the brand's hex values;
  none are written by hand here.
- The monospace choice between JetBrains Mono and Cascadia Code is confirmed by
  rendering tests on the real text stack, not by this guide.
- HDR handling is specified for the reference-white rule only; tone mapping of
  media is Phase 10's question.
- Program icons for the first-party apps do not exist yet; the brand's
  instrument style is the specification for drawing them.

## 17. The decision

D-321 records this guide's one decision: one toolkit and one style source, with
tokens as the OS API, three independent preferences, the brand's palette and
faces adopted as the OS's own, and translucency confined to transient
surfaces. Its alternatives, per-app themes and a cascading style system, are
the two ways every desktop in `018` grew two of everything.
