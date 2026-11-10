use super::{Caught, Mutation};

pub(super) const MUTATIONS: &[Mutation] = &[
    Mutation {
        id: 112,
        file: "libs/fat32/src/lib.rs",
        find: "        if sector[510] != 0x55 || sector[511] != 0xAA {",
        replace: "        if sector[510] != 0x55 && sector[511] != 0xAA {",
        breaks: "half of a FAT boot signature is accepted as the whole signature",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_boot_sector_that_is_not_one_is_refused",
        },
    },
    Mutation {
        id: 113,
        file: "libs/fat32/src/lib.rs",
        find: "                + byte / SECTOR_BYTES as u64,",
        replace: "                + 0,",
        breaks: "FAT entries past the first table sector are read from sector zero",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_boot_sector_is_read_where_the_layout_puts_its_fields",
        },
    },
    Mutation {
        id: 114,
        file: "libs/fat32/src/lib.rs",
        find: "        wanted\n            .iter()\n            .zip(self.name())\n            .all(|(wanted, observed)| upper(*wanted) == upper(*observed))",
        replace: "        wanted\n            .iter()\n            .zip(self.name())\n            .all(|_| true)",
        breaks: "a same-length different FAT name is accepted as a match",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_name_is_matched_without_regard_to_case",
        },
    },
    Mutation {
        id: 115,
        file: "libs/fat32/src/lib.rs",
        find: "        if slot[start + index] != b' ' {\n            return None;\n        }",
        replace: "        if false && slot[start + index] != b' ' {\n            return None;\n        }",
        breaks: "a sparse FAT extension loses its only non-space byte",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::an_eight_three_name_is_unpacked",
        },
    },
    Mutation {
        id: 117,
        file: "libs/fat32/src/lib.rs",
        find: "                done += take;",
        replace: "                done = 0;",
        breaks: "a FAT file read makes no forward byte progress",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::file_reads_match_the_concatenated_sector_bytes_at_every_boundary",
        },
    },
    Mutation {
        id: 118,
        file: "libs/fat32/src/lib.rs",
        find: "                if !entry.is_directory() {\n                    return Err(FatError::WrongKind);\n                }\n                cluster = entry.cluster;",
        replace: "                cluster = entry.cluster;",
        breaks: "path resolution descends through a non-directory component",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::path_resolution_descends_only_through_directory_components",
        },
    },
    // Admit an empty read to the zero-based NLB encoder. In release that
    // underflows to the largest command rather than panicking.
    Mutation {
        id: 243,
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks == 0 {\n            return Err(TransferError::Empty);\n        }",
        replace: "        if false && blocks == 0 {\n            return Err(TransferError::Empty);\n        }",
        breaks: "an empty transfer has no nlb encoding",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::an_empty_transfer_has_no_nlb_encoding",
        },
    },
    // Admit the first count whose zero-based encoding no longer fits the
    // command's sixteen-bit NLB field.
    Mutation {
        id: 244,
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks > MAX_TRANSFER_BLOCKS {\n            return Err(TransferError::TooManyBlocks);\n        }",
        replace: "        if false && blocks > MAX_TRANSFER_BLOCKS {\n            return Err(TransferError::TooManyBlocks);\n        }",
        breaks: "the nlb field has one exact maximum block count",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_nonempty_nlb_count_has_one_exact_boundary",
        },
    },
    // Wrap the LBA end back into the namespace instead of refusing the
    // overflow which would make a controller reach an unrelated block.
    Mutation {
        id: 245,
        file: "libs/nvme/src/lib.rs",
        find: "        let Some(end) = lba.checked_add(blocks as u64) else {\n            return Err(TransferError::OutsideNamespace);\n        };",
        replace: "        let end = lba.wrapping_add(blocks as u64);",
        breaks: "a transfer extent cannot wrap into its namespace",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_transfer_never_crosses_or_wraps_its_namespace",
        },
    },
    // Let the device write one byte past the allocation that proves the DMA
    // extent. Exact-size and short buffers differ only at this comparison.
    Mutation {
        id: 246,
        file: "libs/nvme/src/lib.rs",
        find: "        if bytes > buffer_bytes {\n            return Err(TransferError::BufferTooSmall);\n        }",
        replace: "        if false && bytes > buffer_bytes {\n            return Err(TransferError::BufferTooSmall);\n        }",
        breaks: "a transfer fits wholly inside its dma allocation",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::transfer_bytes_must_fit_the_dma_buffer",
        },
    },
    // Treat a zeroed Identify response as a usable namespace. There is no LBA
    // zero when NSZE is zero, however plausible the remaining format looks.
    Mutation {
        id: 248,
        file: "libs/nvme/src/lib.rs",
        find: "        if blocks == 0 {\n            return Err(NamespaceError::Empty);\n        }",
        replace: "        if false && blocks == 0 {\n            return Err(NamespaceError::Empty);\n        }",
        breaks: "a namespace has at least one logical block",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_namespace_has_at_least_one_logical_block",
        },
    },
    // Accept the extended format table even though this host never enabled
    // LBAFEE and copied only the sixteen-entry prefix it negotiated.
    Mutation {
        id: 249,
        file: "libs/nvme/src/lib.rs",
        find: "        if last_format >= MAX_LBA_FORMATS {\n            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));\n        }",
        replace: "        if false && last_format >= MAX_LBA_FORMATS {\n            return Err(NamespaceError::TooManyFormats(u16::from(last_format) + 1));\n        }",
        breaks: "only negotiated lba formats are accepted",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Ignore FLBAS's reserved bit, making a malformed selector indistinguishable
    // from a format this host knows how to interpret.
    Mutation {
        id: 250,
        file: "libs/nvme/src/lib.rs",
        find: "        if formatted & 0x80 != 0 {\n            return Err(NamespaceError::ReservedFormatBits(formatted));\n        }",
        replace: "        if false && formatted & 0x80 != 0 {\n            return Err(NamespaceError::ReservedFormatBits(formatted));\n        }",
        breaks: "reserved formatted lba selector bits are refused",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Select a packed table entry beyond zero-based NLBAF. Its bytes are not a
    // supported format even when the fixed Identify buffer happens to hold it.
    Mutation {
        id: 251,
        file: "libs/nvme/src/lib.rs",
        find: "        if selected > last_format {\n            return Err(NamespaceError::FormatOutsideList {\n                selected,\n                formats: last_format + 1,\n            });\n        }",
        replace: "        if false && selected > last_format {\n            return Err(NamespaceError::FormatOutsideList {\n                selected,\n                formats: last_format + 1,\n            });\n        }",
        breaks: "the formatted lba index is inside the reported table",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_format_selector_stays_inside_the_negotiated_table",
        },
    },
    // Interpret reserved LBAF control bits as relative-performance bits. The
    // low two are descriptive; every higher bit is an unknown layout contract.
    Mutation {
        id: 252,
        file: "libs/nvme/src/lib.rs",
        find: "        if control & 0xfc != 0 {\n            return Err(NamespaceError::ReservedLbaFormatBits(control));\n        }",
        replace: "        if false && control & 0xfc != 0 {\n            return Err(NamespaceError::ReservedLbaFormatBits(control));\n        }",
        breaks: "every lba format control bit has an explicit policy",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_lba_format_control_value_has_one_explicit_policy",
        },
    },
    // Drop per-LBA metadata from the transfer length and pointer contract.
    // Even one byte makes the controller's data shape differ from the host's.
    Mutation {
        id: 253,
        file: "libs/nvme/src/lib.rs",
        find: "        if metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        replace: "        if false && metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        breaks: "selected lba metadata is never silently discarded",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Confuse support capabilities with active metadata settings. A device may
    // advertise both transfer forms while the selected LBAF uses no metadata.
    Mutation {
        id: 254,
        file: "libs/nvme/src/lib.rs",
        find: "        if metadata != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        replace: "        if metadata != 0 || identify[27] != 0 || identify[28] != 0 {\n            return Err(NamespaceError::Metadata(metadata));\n        }",
        breaks: "metadata capabilities do not enable metadata settings",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Read protected blocks without providing or checking their protection
    // information, turning integrity errors into ordinary data.
    Mutation {
        id: 255,
        file: "libs/nvme/src/lib.rs",
        find: "        if protection != 0 {\n            return Err(NamespaceError::Protection(protection));\n        }",
        replace: "        if false && protection != 0 {\n            return Err(NamespaceError::Protection(protection));\n        }",
        breaks: "enabled protection information is never silently discarded",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_metadata_and_protection_setting_is_explicit",
        },
    },
    // Admit every exponent at or above 512 bytes, restoring the old trust in a
    // byte which may be 32 and which the only storage caller cannot consume.
    Mutation {
        id: 256,
        file: "libs/nvme/src/lib.rs",
        find: "        if block_shift != SUPPORTED_BLOCK_SHIFT {\n            return Err(NamespaceError::BlockSize(block_shift));\n        }",
        replace: "        if block_shift < SUPPORTED_BLOCK_SHIFT {\n            return Err(NamespaceError::BlockSize(block_shift));\n        }",
        breaks: "every lbads byte has one explicit supported policy",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_lbads_value_has_one_explicit_policy",
        },
    },
    // A disk of identical blocks hides a dropped LBA, so the namespace gives
    // every block a distinct signature (D-169).
    Mutation {
        id: 258,
        file: "libs/nvme/src/lib.rs",
        find: "        command.words[10] = lba as u32;
        command.words[11] = (lba >> 32) as u32;",
        replace: "        command.words[10] = 0;
        command.words[11] = 0;",
        breaks: "a read returns the block that was asked for",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_command_puts_every_field_where_the_specification_says",
        },
    },
    // The block count sent unadjusted, so one block becomes two. The first
    // block is still exactly right; only the bytes past it say so.
    Mutation {
        id: 259,
        file: "libs/nvme/src/lib.rs",
        find: "        command.words[12] = transfer.zero_based_blocks() as u32;",
        replace: "        command.words[12] = transfer.zero_based_blocks() as u32 + 1;",
        breaks: "every count in this specification is zero-based",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::a_command_puts_every_field_where_the_specification_says",
        },
    },
    // A missing cluster bias reads valid sectors belonging to another file,
    // so the image corpus distinguishes adjacent clusters (D-170).
    Mutation {
        id: 262,
        file: "libs/fat32/src/lib.rs",
        find: "        let index = (cluster - FIRST_DATA_CLUSTER) as u64;",
        replace: "        let index = cluster as u64;",
        breaks: "a cluster number is biased by two",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_clusters_sector_is_biased_by_two",
        },
    },
    // A read bounded by the cluster rather than by the file, which hands back
    // the last cluster's tail: its previous occupant's data.
    Mutation {
        id: 263,
        file: "libs/fat32/src/lib.rs",
        find: "        let wanted = into.len().min((entry.size - offset) as usize);",
        replace: "        let wanted = into.len();",
        breaks: "a read stops at the end of the file",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // The deleted marker taken for the end of the directory, which loses every
    // file below it. Only reachable because the corpus deletes one on purpose.
    Mutation {
        id: 264,
        file: "libs/fat32/src/lib.rs",
        find: "    if slot[0] == 0xE5 {
        return Slot::Deleted;
    }",
        replace: "    if slot[0] == 0xE5 {
        return Slot::End;
    }",
        breaks: "a deleted entry is not the end of a directory",
        reddens: Caught::Check("the kernel reads the file the host wrote"),
    },
    // Treating `..` as a name lets a caller ask the resolver to escape its
    // starting handle, contrary to `012` section 5 (D-171).
    Mutation {
        id: 266,
        file: "libs/path-walk/src/lib.rs",
        find: "        if component == b\"..\" {",
        replace: "        if false {",
        breaks: "a resolver cannot be asked to ascend",
        reddens: Caught::Check("no path can name anything above the handle"),
    },
    // Ascending past the start clamped rather than refused, which silently
    // answers a different question than the caller asked.
    Mutation {
        id: 267,
        file: "libs/path-walk/src/lib.rs",
        find: "            depth = depth.checked_sub(1).ok_or(PathError::Escapes)?;",
        replace: "            depth = depth.saturating_sub(1);",
        breaks: "an escape is refused rather than clamped",
        reddens: Caught::Check("no path can name anything above the handle"),
    },
    // A component that is not a directory walked through, so a file's bytes
    // are read as directory entries — with the caller choosing the bytes.
    Mutation {
        id: 268,
        file: "libs/fat32/src/lib.rs",
        find: "                if !entry.is_directory() {
                    return Err(FatError::WrongKind);
                }",
        replace: "                if false {
                    return Err(FatError::WrongKind);
                }",
        breaks: "a file cannot be walked through",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::path_resolution_descends_only_through_directory_components",
        },
    },
    Mutation {
        id: 356,
        file: "libs/fat32/src/lib.rs",
        find: "#![cfg_attr(\n    not(test),\n    deny(clippy::expect_used, clippy::panic, clippy::unwrap_used)\n)]",
        replace: "",
        breaks: "a hostile-input parser regains expect, unwrap and panic shortcuts",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "boundary::tests::target_boundaries_keep_panicking_shortcuts_out",
        },
    },
    Mutation {
        id: 358,
        file: "libs/fat32/src/lib.rs",
        find: "        if first_data_sector >= total_sectors as u64 {",
        replace: "        if false && first_data_sector >= total_sectors as u64 {",
        breaks: "a FAT table extent at or beyond the volume is accepted",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::impossible_volume_geometry_is_refused_before_arithmetic",
        },
    },
    Mutation {
        id: 359,
        file: "libs/fat32/src/lib.rs",
        find: "        if (root_cluster - FIRST_DATA_CLUSTER) as u64 >= data_clusters {",
        replace: "        if false && (root_cluster - FIRST_DATA_CLUSTER) as u64 >= data_clusters {",
        breaks: "a FAT root directory may begin outside the complete data clusters",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_root_cluster_must_lie_inside_the_data_area",
        },
    },
    Mutation {
        id: 360,
        file: "libs/fat32/src/lib.rs",
        find: "        if data_clusters + FIRST_DATA_CLUSTER as u64 > fat_entries {",
        replace: "        if false && data_clusters + FIRST_DATA_CLUSTER as u64 > fat_entries {",
        breaks: "a FAT too short to index every advertised data cluster is accepted",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::the_fat_indexes_every_advertised_data_cluster",
        },
    },
    Mutation {
        id: 361,
        file: "libs/fat32/src/lib.rs",
        find: "        if data_clusters < MIN_FAT32_CLUSTERS || data_clusters > MAX_FAT32_CLUSTERS {",
        replace: "        if false || data_clusters > MAX_FAT32_CLUSTERS {",
        breaks: "a FAT12 or FAT16 cluster count is accepted as FAT32",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_fat32_volume_has_at_least_the_fat32_cluster_count",
        },
    },
    Mutation {
        id: 362,
        file: "libs/fat32/src/lib.rs",
        find: "        if data_clusters < MIN_FAT32_CLUSTERS || data_clusters > MAX_FAT32_CLUSTERS {",
        replace: "        if data_clusters < MIN_FAT32_CLUSTERS || false {",
        breaks: "a FAT32 data domain reaches reserved cluster numbers",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_fat32_volume_stops_before_reserved_cluster_numbers",
        },
    },
    Mutation {
        id: 363,
        file: "libs/fat32/src/lib.rs",
        find: "            || (extended_flags & MIRRORING_DISABLED == 0 && selector != 0)",
        replace: "            || false",
        breaks: "a mirrored FAT32 volume accepts a meaningless inactive-table selector",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::extended_flags_reject_reserved_bits_and_meaningless_selectors",
        },
    },
    Mutation {
        id: 364,
        file: "libs/fat32/src/lib.rs",
        find: "            || active_fat >= fats",
        replace: "            || false",
        breaks: "an active FAT selector may name a table the volume does not have",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::an_active_fat_must_name_an_existing_copy",
        },
    },
    Mutation {
        id: 365,
        file: "libs/fat32/src/lib.rs",
        find: "                + self.active_fat as u64 * self.sectors_per_fat as u64",
        replace: "                + self.active_fat as u64 * 0",
        breaks: "FAT reads ignore the selected active table",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::extended_flags_choose_the_fat_that_is_read",
        },
    },
    Mutation {
        id: 366,
        file: "libs/fat32/src/lib.rs",
        find: "        if let Next::Cluster(cluster) = next\n            && !self.bpb.holds(cluster)\n        {\n            return Err(FatError::BadCluster(cluster));\n        }",
        replace: "",
        breaks: "a FAT chain link may name a marker or a cluster outside this volume",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_chain_link_must_name_a_cluster_in_this_volume",
        },
    },
    Mutation {
        id: 367,
        file: "libs/fat32/src/lib.rs",
        find: "        let skip_clusters = offset / cluster_bytes;",
        replace: "        let skip_clusters = offset;",
        breaks: "a FAT byte offset is mistaken for a cluster count",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_maximum_file_offset_does_not_overflow_chain_progress",
        },
    },
    Mutation {
        id: 368,
        file: "libs/fat32/src/lib.rs",
        find: "    const fn chain_limit(&self) -> u32 {\n        self.bpb.clusters()\n    }",
        replace: "    const fn chain_limit(&self) -> u32 {\n        1 << 24\n    }",
        breaks: "a FAT chain walk is bounded by the volume that could hold it",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_file_skip_past_the_volume_is_refused_before_walking",
        },
    },
    Mutation {
        id: 370,
        file: "libs/fat32/src/lib.rs",
        find: "                Next::Cluster(next) => {\n                    let Some(visited) = cursor.visited.checked_add(1) else {\n                        return Err(FatError::Unbounded);\n                    };\n                    if visited >= self.bpb.directory_cluster_limit() {\n                        return Err(FatError::Unbounded);\n                    }\n                    let hare = self.advance_cycle_hare(cursor.hare)?;\n                    if hare == Some(next) {\n                        return Err(FatError::Unbounded);\n                    }\n                    cursor.cluster = next;\n                    cursor.sector = 0;\n                    cursor.visited = visited;\n                    cursor.hare = hare;\n                }\n                Next::End => {\n                    cursor.done = true;\n                    return Ok(DirectoryChunk::EMPTY);\n                }",
        replace: "                Next::Cluster(_) | Next::End => {\n                    cursor.done = true;\n                    return Ok(DirectoryChunk::EMPTY);\n                }",
        breaks: "a FAT directory walk silently stops after its first cluster",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "image::foreign::both_readers_agree_on_the_adversarial_foreign_layout",
        },
    },
    Mutation {
        id: 420,
        file: "libs/elf64/src/lib.rs",
        find: "        match self.offset.checked_add(self.file_size) {\n            Some(end) => end <= bytes as u64,\n            None => false,\n        }",
        replace: "        self.offset.wrapping_add(self.file_size) <= bytes as u64",
        breaks: "a segment extent wraps around into the elf image",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::a_loader_can_ask_whether_the_contents_fit",
        },
    },
    Mutation {
        id: 423,
        file: "libs/path-walk/src/lib.rs",
        find: "            depth = depth.checked_sub(1).ok_or(PathError::Escapes)?;",
        replace: "            depth = depth.saturating_sub(1);",
        breaks: "ascending above a directory handle is clamped to its root",
        reddens: Caught::HostTest {
            package: "path-walk",
            name: "tests::ascending_past_the_start_is_refused_not_clamped",
        },
    },
    Mutation {
        id: 444,
        file: "libs/elf64/src/lib.rs",
        find: "            offset: read_u64(header, 40)?,",
        replace: "            offset: read_u64(header, 32)?,",
        breaks: "the section table offset is read from the program table field",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::the_section_table_fields_are_where_the_file_header_puts_them",
        },
    },
    // The property tier's own falsifiability (D-242): the defect must
    // survive every generated case, not one a unit test names.
    Mutation {
        id: 549,
        file: "libs/path-walk/src/lib.rs",
        find: "        if component == b\".\" {",
        replace: "        if false && component == b\".\" {",
        breaks: "a bare dot is a no-op rather than a component the walker descends into",
        reddens: Caught::HostTest {
            package: "path-walk",
            name: "no_split_output_can_name_an_ancestor",
        },
    },
    Mutation {
        id: 552,
        file: "libs/nvme/src/lib.rs",
        find: "            && address.checked_add(len).is_some()",
        replace: "            && true",
        breaks: "a command buffer's contiguous physical extent can wrap the address space",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::identify_binds_one_complete_aligned_page",
        },
    },
    Mutation {
        id: 556,
        file: "libs/path-walk/src/lib.rs",
        find: "#[cfg(test)]\nmod tests",
        replace: "/// Whether `path` begins as a relative path.\npub const fn is_relative(path: &[u8]) -> bool {\n    !path.is_empty() && path[0] != SEPARATOR\n}\n\n#[cfg(test)]\nmod tests",
        breaks: "a weaker duplicate path predicate re-enters the public compiler surface",
        reddens: Caught::ApiAudit("fn is_relative"),
    },
    Mutation {
        id: 576,
        file: "libs/elf64/src/lib.rs",
        find: "            Self::NotAnElf => f.write_str(\"does not begin with the elf magic\"),",
        replace: "            Self::NotAnElf => f.write_str(\"\"),",
        breaks: "an ELF magic refusal loses its diagnostic reason",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::diagnostic_messages_keep_their_reason_and_values",
        },
    },
    Mutation {
        id: 577,
        file: "libs/elf64/src/lib.rs",
        find: "    pub const fn flags(self) -> u32 {\n        self.flags\n    }",
        replace: "    pub const fn flags(self) -> u32 {\n        0\n    }",
        breaks: "ELF segment metadata erases every permission bit",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 578,
        file: "libs/elf64/src/lib.rs",
        find: "    /// Where in the file its contents start.\n    pub const fn offset(self) -> u64 {\n        self.offset\n    }",
        replace: "    /// Where in the file its contents start.\n    pub const fn offset(self) -> u64 {\n        0\n    }",
        breaks: "ELF segment metadata erases the file offset",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 579,
        file: "libs/elf64/src/lib.rs",
        find: "        self.flags & Self::READ != 0",
        replace: "        self.flags | Self::READ != 0",
        breaks: "every ELF segment is reported readable",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 580,
        file: "libs/elf64/src/lib.rs",
        find: "        self.file_size > self.memory_size",
        replace: "        self.file_size >= self.memory_size",
        breaks: "an ELF segment whose contents exactly fit is rejected as oversized",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 581,
        file: "libs/elf64/src/lib.rs",
        find: "    pub const fn segment_count(&self) -> u16 {\n        self.program_count\n    }",
        replace: "    pub const fn segment_count(&self) -> u16 {\n        1\n    }",
        breaks: "every ELF image reports one program segment",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_metadata_and_permission_boundaries_are_exact",
        },
    },
    Mutation {
        id: 600,
        file: "libs/elf64/src/lib.rs",
        find: "    pub const fn has_valid_alignment(self) -> bool {\n        self.alignment <= 1\n            || self.alignment.is_power_of_two()\n                && self.address % self.alignment == self.offset % self.alignment\n    }",
        replace: "    pub const fn has_valid_alignment(self) -> bool {\n        true\n    }",
        breaks: "ELF accepts non-power-of-two and file-incongruent segment alignment",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::segment_alignment_is_power_of_two_and_congruent",
        },
    },
    Mutation {
        id: 640,
        file: "libs/elf64/src/lib.rs",
        find: "        if u32::from(identification_version) != CURRENT_VERSION {",
        replace: "        if false && u32::from(identification_version) != CURRENT_VERSION {",
        breaks: "ELF accepts an invalid identification version",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        id: 641,
        file: "libs/elf64/src/lib.rs",
        find: "        if kind != EXECUTABLE {",
        replace: "        if false && kind != EXECUTABLE {",
        breaks: "the loader accepts a non-executable ELF object type",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        id: 642,
        file: "libs/elf64/src/lib.rs",
        find: "        if version != CURRENT_VERSION {",
        replace: "        if false && version != CURRENT_VERSION {",
        breaks: "ELF accepts an invalid file-header version",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        id: 643,
        file: "libs/elf64/src/lib.rs",
        find: "        if header_size as usize != HEADER_BYTES {",
        replace: "        if false && header_size as usize != HEADER_BYTES {",
        breaks: "ELF accepts a non-ELF64 file-header size",
        reddens: Caught::HostTest {
            package: "elf64",
            name: "tests::every_required_header_identity_field_is_validated",
        },
    },
    Mutation {
        id: 707,
        file: "libs/nvme/src/lib.rs",
        find: "        if namespace == 0 || !Self::contiguous_extent(address, IDENTIFY_TRANSFER_BYTES, page_size) {",
        replace: "        if namespace == 0 {",
        breaks: "identify can bind a partial, unaligned or wrapping DMA page",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::identify_binds_one_complete_aligned_page",
        },
    },
    Mutation {
        id: 708,
        file: "libs/nvme/src/lib.rs",
        find: "            || !Self::contiguous_extent(address, transfer.bytes(), page_size)",
        replace: "            || false",
        breaks: "read can bind bytes outside its validated contiguous DMA page",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::read_binds_its_validated_extent_to_one_page",
        },
    },
    Mutation {
        id: 709,
        file: "libs/nvme/src/lib.rs",
        find: "        if queue == 0 || !Self::contiguous_extent(address, capacity.completion_bytes(), page_size) {",
        replace: "        if queue == 0 {",
        breaks: "completion-queue creation can bind an incomplete contiguous extent",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::queue_commands_bind_the_complete_contiguous_extent",
        },
    },
    Mutation {
        id: 710,
        file: "libs/nvme/src/lib.rs",
        find: "        self.state = DmaOwnership::InFlight;\n        Some(handoff(DmaPublication { _state: self }))",
        replace: "        let outcome = handoff(DmaPublication { _state: self });\n        self.state = DmaOwnership::InFlight;\n        Some(outcome)",
        breaks: "doorbell handoff observes DMA memory before publication closes cancellation",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::publication_closes_the_unpublished_cancellation_door",
        },
    },
    Mutation {
        id: 711,
        file: "libs/nvme/src/lib.rs",
        find: "        if self.state != DmaOwnership::InFlight {",
        replace: "        if self.state == DmaOwnership::Idle {",
        breaks: "DMA completion accepts memory that was never published to the device",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::publication_closes_the_unpublished_cancellation_door",
        },
    },
    Mutation {
        id: 830,
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(PartialEq, Eq, Debug)]\npub struct DmaState {",
        replace: "#[derive(Clone, PartialEq, Eq, Debug)]\npub struct DmaState {",
        breaks: "DMA ownership authority can be duplicated before publication or completion",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        id: 920,
        file: "libs/nvme/src/lib.rs",
        find: "pub const fn doorbell(capabilities: Capabilities, queue: u16, doorbell: Doorbell) -> Option<usize> {",
        replace: "pub const fn doorbell(capabilities: Capabilities, queue: u16, doorbell: bool) -> Option<usize> {",
        breaks: "NVMe submission/completion doorbell selection becomes positional boolean state",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 921,
        file: "libs/nvme/src/lib.rs",
        find: "    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        replace: "    /// Recover hardware quiescence from an unnamed observation.\n    pub fn quiesce(&mut self, verified: bool) {\n        if verified {\n            self.close();\n        }\n    }\n\n    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        breaks: "pure NVMe DMA state regains boolean hardware-quiescence authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 922,
        file: "libs/nvme/src/lib.rs",
        find: "    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        replace: "    /// Recover retained hardware ownership from an unnamed observation.\n    pub fn retain_unquiesced(&mut self, retained: bool) {\n        if retained {\n            self.close();\n        }\n    }\n\n    /// Permanently refuse another publication on this controller.\n    pub fn close(&mut self) {",
        breaks: "pure NVMe DMA state regains boolean retention authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 923,
        file: "libs/nvme/src/lib.rs",
        find: "    pub fn close(&mut self) {",
        replace: "    pub fn close(&mut self, _closed: bool) {",
        breaks: "NVMe DMA closure regains a redundant unnamed state boolean",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 957,
        file: "libs/nvme/src/lib.rs",
        find: "        self.state = DmaOwnership::Closed;",
        replace: "        self.state = DmaOwnership::Idle;",
        breaks: "closing pure DMA state reopens publication instead of making it terminal",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::closed_dma_state_cannot_publish_or_complete",
        },
    },
    Mutation {
        id: 986,
        file: "libs/nvme/src/lib.rs",
        find: "    pub fn close(&mut self) {",
        replace: "    pub fn quiesce(&mut self) {",
        breaks: "pure DMA state renames terminal closure into a hardware-recycling claim",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::typed_arguments::selected_state_selectors_use_semantic_types",
        },
    },
    Mutation {
        id: 998,
        file: "libs/fat32/src/lib.rs",
        find: "    const fn sector_of(self, cluster: u32) -> Result<u64, FatError> {",
        replace: "    pub const fn sector_of(self, cluster: u32) -> Result<u64, FatError> {",
        breaks: "FAT cluster-to-sector internals regrow as speculative public arithmetic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 999,
        file: "libs/fat32/src/lib.rs",
        find: "    const fn table_position(self, cluster: u32) -> (u64, usize) {",
        replace: "    pub const fn table_position(self, cluster: u32) -> (u64, usize) {",
        breaks: "FAT table-position internals regrow as speculative public arithmetic",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1000,
        file: "libs/fat32/src/lib.rs",
        find: "const fn decode_entry(raw: u32) -> Next {",
        replace: "pub const fn decode_entry(raw: u32) -> Next {",
        breaks: "raw FAT entry decoding regrows as a speculative public policy door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1001,
        file: "libs/fat32/src/lib.rs",
        find: "const fn decode_slot(slot: &[u8; ENTRY_BYTES]) -> Slot {",
        replace: "pub const fn decode_slot(slot: &[u8; ENTRY_BYTES]) -> Slot {",
        breaks: "raw FAT directory-slot decoding regrows as a speculative public policy door",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1003,
        file: "libs/path-walk/src/lib.rs",
        find: "fn is_a_name(component: &[u8]) -> bool {",
        replace: "pub fn is_a_name(component: &[u8]) -> bool {",
        breaks: "pathname component policy regrows as a speculative public predicate",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::address_space::speculative_and_callback_authority_does_not_regrow",
        },
    },
    Mutation {
        id: 1179,
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(PartialEq, Eq, Debug)]\npub struct DmaState {",
        replace: "#[derive(Clone, Copy, PartialEq, Eq, Debug)]\npub struct DmaState {",
        breaks: "DMA ownership state can be copied as well as cloned",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        id: 1180,
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug)]\npub struct DmaPublication<'state> {",
        replace: "#[derive(Debug, Clone)]\npub struct DmaPublication<'state> {",
        breaks: "a device-publication proof can be cloned while its source remains live",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        id: 1181,
        file: "libs/nvme/src/lib.rs",
        find: "#[derive(Debug)]\npub struct DmaPublication<'state> {",
        replace: "#[derive(Debug, Clone, Copy)]\npub struct DmaPublication<'state> {",
        breaks: "a device-publication proof can be copied without consuming its authority",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::apic::dma_ownership_and_publication_authorities_are_linear",
        },
    },
    Mutation {
        id: 1278,
        file: "libs/nvme/src/lib.rs",
        find: "        if cc & configuration::ENABLE != 0 || csts & status::READY != 0 {",
        replace: "        if csts & status::READY != 0 {",
        breaks: "NVMe quiescence ignores an enabled controller",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::only_disabled_and_not_ready_registers_pass_dma_classification",
        },
    },
    Mutation {
        id: 1279,
        file: "libs/nvme/src/lib.rs",
        find: "        if cc & configuration::ENABLE != 0 || csts & status::READY != 0 {",
        replace: "        if cc & configuration::ENABLE != 0 {",
        breaks: "NVMe quiescence ignores a controller that remains ready",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::only_disabled_and_not_ready_registers_pass_dma_classification",
        },
    },
    Mutation {
        id: 1280,
        file: "libs/nvme/src/lib.rs",
        find: "        match DisabledRegisters::from_bits(cc, csts) {",
        replace: "        match DisabledRegisters::from_bits(0, 0) {",
        breaks: "unverified NVMe register state can discard quarantine authority",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_unverified_preparation_access_failure_preserves_its_quarantine_owner",
        },
    },
    Mutation {
        id: 1281,
        file: "libs/nvme/src/lib.rs",
        find: "impl<T> DisabledPreparation<T> {\n    /// Attempt one disabled-controller access with exact retirement fallback.\n    pub fn access<U, E>(\n        mut self,\n        operation: impl FnOnce(&mut T) -> Result<U, E>,\n    ) -> Result<(Self, U), PreparationFailure<Self, E>> {\n        match operation(&mut self.owner) {\n            Ok(value) => Ok((self, value)),\n            Err(reason) => Err(PreparationFailure {\n                preparation: self,\n                reason,\n            }),\n        }\n    }",
        replace: "impl<T> DisabledPreparation<T> {\n    /// Attempt one disabled-controller access with exact retirement fallback.\n    pub fn access<U, E>(\n        mut self,\n        operation: impl FnOnce(&mut T) -> Result<U, E>,\n    ) -> Result<(Self, U), PreparationFailure<Self, E>> {\n        match operation(&mut self.owner) {\n            Ok(value) => Ok((self, value)),\n            Err(_) => panic!(\"mutant discarded disabled-controller cleanup authority\"),\n        }\n    }",
        breaks: "a disabled-controller access refusal discards exact retirement authority",
        reddens: Caught::HostTest {
            package: "nvme",
            name: "tests::every_disabled_preparation_access_failure_returns_the_exact_retirement_owner",
        },
    },
    Mutation {
        id: 1383,
        file: "libs/fat32/src/lib.rs",
        find: "#[derive(Clone, Copy, PartialEq, Eq, Debug)]\n#[non_exhaustive]\npub enum FatError {",
        replace: "#[derive(Clone, Copy, PartialEq, Eq, Debug)]\npub enum FatError {",
        breaks: "a public error enum can regain an exhaustive downstream variant contract",
        reddens: Caught::HostTest {
            package: "xtask",
            name: "contracts::errors::public_error_enums_have_stable_standard_contracts",
        },
    },
    Mutation {
        id: 1384,
        file: "libs/fat32/src/lib.rs",
        find: "            Self::Unreadable => f.write_str(\"the sector source refused a read\"),",
        replace: "            Self::Unreadable => f.write_str(\"unreadable\"),",
        breaks: "a FAT refusal loses its actionable sector-source cause",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::every_fat_error_describes_its_cause",
        },
    },
    Mutation {
        id: 1385,
        file: "libs/path-walk/src/lib.rs",
        find: "            Self::Empty => \"the path is empty\",",
        replace: "            Self::Empty => \"empty\",",
        breaks: "a path refusal loses the argument it describes",
        reddens: Caught::HostTest {
            package: "path-walk",
            name: "tests::every_path_error_describes_its_cause",
        },
    },
    Mutation {
        id: 1534,
        file: "libs/fat32/src/lib.rs",
        find: "                    if visited >= self.bpb.directory_cluster_limit() {",
        replace: "                    if visited >= self.chain_limit() {",
        breaks: "a directory walk is bounded by the volume, not the entry cap",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_huge_acyclic_directory_stops_at_the_entry_cap",
        },
    },
    Mutation {
        id: 1535,
        file: "libs/fat32/src/lib.rs",
        find: "                    let hare = self.advance_cycle_hare(cursor.hare)?;\n                    if hare == Some(next) {\n                        return Err(FatError::Unbounded);\n                    }\n",
        replace: "                    let hare = self.advance_cycle_hare(cursor.hare)?;\n",
        breaks: "a directory walk never checks its chain for a cycle",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_cyclic_directory_chain_is_caught_by_the_hare_not_the_volume",
        },
    },
    Mutation {
        id: 2460,
        file: "libs/fat32/src/lib.rs",
        find: ".ok_or(FatError::Malformed)?;",
        replace: ".ok_or(FatError::BadCluster(0))?;",
        breaks: "a wrapped sector address is reported as cluster 0",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_partition_base_that_wraps_is_malformed",
        },
    },
    Mutation {
        id: 2461,
        file: "libs/fat32/src/lib.rs",
        find: "Ok(Next::End | Next::Bad | Next::Free) | Err(FatError::BadCluster(_)) => None,",
        replace: "Ok(Next::End | Next::Bad | Next::Free) => None,",
        breaks: "the cycle check fails a read on a link the read never follows",
        reddens: Caught::HostTest {
            package: "fat32",
            name: "tests::a_reserved_link_beyond_the_last_cluster_does_not_hide_it",
        },
    },
];
