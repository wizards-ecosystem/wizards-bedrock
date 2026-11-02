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

Nothing. The census is recorded below. The one remaining warning is
`vtd::second_level::pte`, which `wost` calls.

## Next steps

1. Do not merge.
2. If `second_level::pte`'s two `bool`s should become a permission enum,
   that edit is in `wost/src/arch/x86_64/vtd.rs` as well as `libs/vtd`.
   This stream did not make it.
3. Re-run the census after stream 1's `clippy.toml` lands on `main`. The
   crate-level `#![warn]` attributes are what make `#[expect]` fulfilled
   before that file exists.

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

## Clippy census after this pass

Against stream 1's table (`CLIPPY_CONF_DIR` pointed at that branch's
`clippy.toml`, `cargo clippy --all-targets` on every `libs/` crate,
`-D unfulfilled_lint_expectations`). Host only. rustc 1.97.1.

| Crate | Remaining warnings |
|---|---:|
| vtd | 1 |
| every other `libs/` crate | 0 |

The one warning is `second_level::pte(paddr, read: bool, write: bool)`:
`fn_params_excessive_bools` at threshold 0. Replacing the bools changes
the signature `wost` calls. Left in place.

`#[allow]` was not used. Stream 1 warns on `allow` via `allow_attributes`.
Proven narrowings and closed enums use `#[expect(..., reason = "...")]`,
and the crate warns that lint so today's gate (which does not load stream
1's `clippy.toml`) still sees the expectation fulfilled.
`avoid-breaking-exported-api` is false only in that `clippy.toml`, so an
`expect` on a public item that the default config hides is unfulfilled
today. Those sites were changed instead of suppressed.

## API changes that do not need `kernel/` edits

Searched `kernel/` and `wost/` before renaming.

- `PageFault::from_user` is now `PageFault::was_user`, matching
  `was_present` and `was_write`. Nothing outside `libs/trap-cause` called it.
- `Class.class` is now `Class.code`. Callers use `Class::is_nvme` and
  `Class::is_bridge`. Nothing outside this crate named the field.

## API changes that would need `kernel/` or `wost/` edits

- `vtd::second_level::pte(paddr, read: bool, write: bool)`. `wost`'s VT-d
  mapper passes `DEVICE_READS` and `DEVICE_WRITES`. Not changed.

## Decisions

- An I/O address outside `u16` is `None` from `Bar::decode`, the same answer
  as an unsupported memory encoding. The probe then fails `Malformed` instead
  of publishing a window. D-168 already sized I/O windows as `u16`; this makes
  the address obey the same bound. No new decision-log entry: `docs/` is
  outside this stream, and the bound was already the type's.
- Do not mark closed protocol enums `#[non_exhaustive]` just to silence
  `exhaustive_enums`. That breaks kernel matches this stream cannot update.
