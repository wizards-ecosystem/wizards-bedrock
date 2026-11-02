# Stream 03 — libs

Owner: `libs/` only, including each lib crate's `Cargo.toml`. No root
`Cargo.toml`, clippy config, CI, `kernel/`, `boot/`, `xtask/`, `tools/`, or
`wost/`.

Branch: `cursor/libs-soundness-and-lints-8fd3`

## Done

- Small mechanical nits: `let`/`else` in `mmio-range::covering`, proptest
  `cases` via `u32::try_from`, ring-proto interleaving asserts name the
  failure, `Key`'s missing-docs expect has a reason, pixel hex literals are
  grouped, and the timebase drift test declares its consts before statements.
- Lossless `as` casts outside `const fn` are `From` (`u64::from`). Casts
  inside `const fn` stay, because `From` is not const-callable on 1.97.1.
- Doc comments backtick the identifiers `doc_markdown` named (`VMware`,
  `x86_64`, `PrintScreen`, `IOTLB_REG`, and the PIT literals). Root
  `clippy.toml` was not edited, so the idents are quoted in the prose.
- `# Errors` on the 16 `Result` functions the census named (`ring-proto`,
  `frame-space`, `nvme`, `x86-apic`, `x86-paging`, `acpi-tables`,
  `pci-config`). `cargo doc --no-deps` on those crates reported no broken links.
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
- VT-d fault reason is bits 103:96. I/O window size is the 16-bit
  complement. A zero-length memory region and a backwards span no longer
  count as coverage. The FAT cycle hare stops on a reserved lookahead
  instead of failing the read, and a sector-number wrap is `Malformed`.
  Left alone: reserved directory-attribute bits and non-zero reserved MADT
  override flags (existing tests treat them as policy), huge-page PAT in
  `Entry::address` (the encoder cannot set it), and `canonical_decimal("0")`
  (the unit test rejects zero on purpose; nothing in the kernel calls it).

## In progress

- Register-width `cast_possible_truncation` (about 120 sites) and
  `exhaustive_enums` (37 closed protocol enums). `#[expect]` is unfulfilled
  unless the lint is enabled, and enabling it crate-wide fails `cargo xtask
  clippy -D warnings` until every site in that crate is answered. Do one
  crate at a time: warn the lint in that crate, `expect` the proven field
  extracts, `TryFrom` where the value can fail.
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
| High | `libs/vtd` `FaultRecording::fault_reason` | FR was read at bits 111:104, which is the PASID value. QEMU places FR at bits 103:96 (`<< 32` in the high qword). Fixed. The old test restated the wrong boundary, so it agreed with the bug. |
| Medium | `libs/pci-config` `io_window_size` | A 16-bit I/O decoder (bits 31:16 hardwired 0) was sized in 32 bits and rejected. `0x0000_FFFC` is 4 ports. Fixed by complementing in 16 bits. |
| Medium | `libs/memory-map` `allows_device_mapping` | A zero-length region overlapped every query that strictly contained its base. Fixed by ignoring `len == 0`, as `covered_by` already does. |
| Low | `libs/memory-map` `covered_by` | `start > end` returned covered. An empty span (`start == end`) stays vacuously covered. Fixed. |
| Medium | `libs/fat32` cycle hare | A reserved link two steps ahead aborted the read before the cluster about to be read was returned. The hare now stops on `BadCluster`; `next_cluster` still errors when that link is followed. |
| Low | `libs/fat32` `Volume::sector` | A base+sector wrap was `BadCluster(0)`. It is `Malformed`. |

## Decisions

- An I/O address outside `u16` is `None` from `Bar::decode`, the same answer
  as an unsupported memory encoding. The probe then fails `Malformed` instead
  of publishing a window. D-168 already sized I/O windows as `u16`; this makes
  the address obey the same bound. No new decision-log entry: `docs/` is
  outside this stream, and the bound was already the type's.
- Do not mark closed protocol enums `#[non_exhaustive]` just to silence
  `exhaustive_enums`. That breaks kernel matches this stream cannot update.
