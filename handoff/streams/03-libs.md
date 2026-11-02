# Stream 03 — libs

Owner: `libs/` only, including each lib crate's `Cargo.toml`. No root
`Cargo.toml`, clippy config, CI, `kernel/`, `boot/`, `xtask/`, `tools/`, or
`wost/`.

Branch: `cursor/libs-soundness-and-lints-8fd3`

## Done

- `Bar::decode` refuses an I/O BAR whose address does not fit the 16-bit port
  space. The old cast kept only bits 15:2, so `0x1_0001` decoded as port 0
  and a later alignment check accepted it. `u16::try_from` is the refusal.
  Signature of `Bar` is unchanged. `From`/`TryFrom` are not const-callable
  here, so the fit check is `address > u16::MAX as u32`.
  `cargo test -p pci-config --lib --locked a_base_address_register_decodes`
  passed (1 test). Full suite not run.

## In progress

- Host clippy census against stream 1's warn table
  (`origin/cursor/beautiful-rust-lints-b7b1`, `CLIPPY_CONF_DIR` only; root
  config is not edited). Clearing those warnings in `libs/`.

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
