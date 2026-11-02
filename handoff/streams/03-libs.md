# Stream 03 — libs

Owner: `libs/` only, including each lib crate's `Cargo.toml`. No root
`Cargo.toml`, clippy config, CI, `kernel/`, `boot/`, `xtask/`, `tools/`, or
`wost/`.

Branch: `cursor/libs-soundness-and-lints-8fd3`

## Done

- Pure methods that return a new `Self` and do not mutate are `#[must_use]`:
  handle-space `reused`/`union`/`intersection`/`difference`, x86-cpuid
  `with_smep`/`with_smap`/`with_umip`, x86-paging `intersection`/`union`/
  `difference`, pci-config `Address::function_base`, pixel `Rgb::blend`,
  x86-apic `ApicBase::enabled`/`x2apic`. Discarding one was a silent no-op.
- `Bar::decode` refuses an I/O BAR whose address does not fit the 16-bit port
  space. The old cast kept only bits 15:2, so `0x1_0001` decoded as port 0
  and a later alignment check accepted it. Decode now returns `None`.
  Signature of `Bar` is unchanged. `From`/`TryFrom` are not const-callable
  here, so the fit check is `address > u16::MAX as u32`.
  `cargo test -p pci-config --lib --locked a_base_address_register_decodes`
  passed (1 test). Full suite not run.

## In progress

- Clearing the host census (320 unique warns, matches stream 1). `#[must_use]`
  is on the pure `Self`-returning builders the census named. `# Errors`,
  register-width casts, and closed-enum `expect`s are next.
- Census command is local only (`CLIPPY_CONF_DIR=/tmp/bedrock-clippy`,
  stream 1's `clippy.toml`). Root config is not edited.

## Next steps

1. Clear the census, smallest crates first. Register-width field extracts get
   a narrow `#[expect(clippy::cast_possible_truncation, reason = "...")]`.
   Values that can actually not fit use `TryFrom`. Lossless `as` becomes
   `From`. Closed hardware enums get
   `#[expect(clippy::exhaustive_enums, reason = "closed encoding")]` rather
   than `#[non_exhaustive]`, because kernel matches are exhaustive and this
   stream does not edit `kernel/`.
2. Keep looking for soundness bugs while touching those crates.
3. Do not merge.

## Bugs found

| Severity | Where | What |
|---|---|---|
| High | `libs/pci-config` `Bar::decode` | I/O BAR bits 31:16 were truncated. A port at `0x10000` became port 0 and could be claimed. Fixed. |

## Decisions

- An I/O address outside `u16` is `None` from `Bar::decode`, the same answer
  as an unsupported memory encoding. The probe then fails `Malformed` instead
  of publishing a window. D-168 already sized I/O windows as `u16`; this makes
  the address obey the same bound. No new decision-log entry: `docs/` is
  outside this stream, and the bound was already the type's.
- Do not mark closed protocol enums `#[non_exhaustive]` just to silence
  `exhaustive_enums`. That breaks kernel matches this stream cannot update.
