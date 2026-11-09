# D-321 One toolkit and one style source, with tokens as the OS API

Status: Accepted, 2026-09-12. Amends D-11 and refines D-19.

**Context.** D-11 chose a GPU-first native toolkit and said nothing about its
style. `.github/BRAND.md` fixed the ecosystem's palette, typefaces, voice and
interface rules on 2026-09-08. `018` found every desktop that grew a second
toolkit or theming system paying for it for years, Windows' three UI
generations and macOS's Liquid Glass as the current cautionary examples, and
the Design Tokens format, OKLCH, libadwaita's accent derivation, Windows'
contrast themes and base24 as settled practice.

**Decision.** One toolkit draws every first-party surface from one token
document per theme in the Design Tokens format with OKLCH values, resolved
global to semantic to component; a theme may set semantic tokens only and is
validated for contrast before use. The brand's palette and faces are the OS's
own: Instrument Sans for the interface, Newsreader for reading, JetBrains
Mono for code, Atkinson Hyperlegible Next for accessibility, all OFL; Tabler
outlines for system icons; the brand's instrument style for program icons.
Appearance, contrast and motion are three independent preferences; the
contrast theme is a separate 7:1 palette with eight roles; reduced motion and
reduced transparency are hard switches. The base material is opaque and
tinted, translucency is confined to transient surfaces, radii are three fixed
tokens, a stroke replaces the key shadow, and the type ramp, spacing grid,
elevation levels, motion curves and chrome measures are the ones
`020-style-guide.md` records. The toolkit's renderer has a CPU backend beside
the GPU one so the bench runs before a GPU driver exists, which amends D-11's
GPU-first sentence into GPU-first with a CPU fallback for the toolkit as well
as the framebuffer.

**Rationale.** A tiny vocabulary is what let Windows' new surfaces feel like
one product cheaply and what let libadwaita apps look like they belong; the
failure in both cases was covering only part of the system. A first-party
toolkit used by every first-party app, with tokens third parties inherit, is
the only design that covers all of it.

**Alternatives.** Per-app themes were rejected on the evidence. A cascading
style system was rejected because the cascade is where consistency goes to
die and where layout becomes circular. Choosing faces outside the OFL was
rejected because the image must bundle them.

**Consequences.** The token document and its contrast checks are host-tested
before the toolkit exists. `020` is listed with `019` among the current-facing
plans. D-11 and D-19 gain amendment notes.
