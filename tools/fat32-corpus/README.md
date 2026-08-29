# Foreign FAT32 corpus

`foreign-fat32.sparse` is a pinned test artifact, not an input to the shipped
image. It closes D-170's single-writer proof gap: the ordinary corpus is made
by the Rust `fatfs` formatter, while this one was formatted and populated by
independent C tools on 2026-08-28.

## Provenance

The tools were extracted without installation from these Ubuntu 26.04 LTS
(`resolute`) archive packages:

| Package | Archive object | SHA-256 | Upstream and license |
|---|---|---|---|
| `dosfstools` 4.2-1.2build1 | `pool/main/d/dosfstools/dosfstools_4.2-1.2build1_amd64.deb` | `6b1b2bba6458ca4444cae467db7faff6fd2f350373a95be23333d76dd019c58f` | `https://github.com/dosfstools/dosfstools`, GPL-3.0-or-later |
| `mtools` 4.0.49-1 | `pool/main/m/mtools/mtools_4.0.49-1_amd64.deb` | `28edc5a0eee3f74a15135ec90ae2dee79270931df1d0c6415ab736b7eed46ad3` | `https://www.gnu.org/software/mtools/`, GPL-3.0-only |

No package, source or executable is redistributed here. The committed artifact
is generated filesystem data whose file contents were authored for this
project and are licensed under the adjacent MIT `LICENSE`.

`mkfs.fat` formatted a zero-filled 64-MiB file with:

```text
mkfs.fat -F 32 -S 512 -s 1 -R 64 -f 2 -n WIZFOREIGN -i 575a5931 foreign-fat32.img
```

`mtools` then created forty numbered root files and three 8-KiB hole files
interleaved with two retained files. After deleting the holes, a temporary
zero-filled file advanced allocation to the volume's end; `FRAG.BIN` then
wrapped into the three separated free runs, and the temporary file was
deleted. It also created `ARCHIVE/INNER.TXT`,
`A Long Foreign Name.txt`, and deleted `R05.TXT` after later root entries
existed. `FRAG.BIN` uses the position-dependent byte formula enforced by
D-207's host test. `fsck.fat -vn` reported 46 files, 117 used clusters and no
error.

The exact expanded 67,108,864-byte image has SHA-256
`06a6ad589c3652731fc60bc669e4e20bc4576da7761c18be0dc9c800fa4d1576`.
`FRAG.BIN` has SHA-256
`22dfc0f06ddd6321556b31b8d82a184f4a7d5d9f1dada1ddca8d380c69f4370c`.

## Sparse envelope

The first line is `wizard-fat32-sparse-v1`. The next three lines state the
expanded byte length, 512-byte sector size and expanded-image SHA-256. Every
remaining line is `sector INDEX HEX`, where `HEX` is one complete non-zero
sector. Every omitted sector is zero. The checked test decoder rejects any
duplicate, malformed, partial or out-of-range record and verifies the expanded
digest before either reader sees the bytes.
