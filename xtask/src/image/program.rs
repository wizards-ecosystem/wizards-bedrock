//! The user program that ships in the image.
//!
//! Twenty-two bytes of machine code inside a minimal ELF64, assembled here
//! rather
//! than compiled. That is a deliberate stopping point: a real userland needs a
//! second crate, a second link address and a fourth pass in the gate's
//! configuration matrix (D-22), and none of that tests the *loader*. What the
//! loader needs is a genuine ELF on a real initrd, and this is the smallest
//! thing that is one (D-100).
//!
//! The layout is written out field by field against the specification, and the
//! test parses it back with `libs/elf64` — the same crate the kernel uses. A
//! builder checked against its own reader would prove nothing; checked against
//! the consumer, it proves the two agree.

use elf64::{HEADER_BYTES, PROGRAM_HEADER_BYTES};

/// Where the program is linked, and where the kernel maps it.
pub const LOAD_ADDRESS: u64 = 0x0000_4000_0000_0000;

/// The program: read one word of argument, then either call the kernel or
/// fault deliberately.
///
/// ```text
///   58                       pop rax                     ; the kernel's word
///   48 85 c0                 test rax, rax
///   74 02                    jz  +2                      ; zero: behave
///   0f 0b                    ud2                         ; anything else: die
///   9c                       pushfq
///   48 81 0c 24 00 00 04 00  or qword ptr [rsp], 0x40000 ; set AC, bit 18
///   9d                       popfq
///   cd 80                    int 0x80
///   eb fe                    jmp .
/// ```
///
/// Two behaviours from one image, chosen by a word the kernel writes onto the
/// program's stack with `copy_to_user` — which is how a program will get its
/// arguments, and means the boundary API is exercised on the way in as well as
/// out (D-102).
///
/// The `ud2` is the point of the second behaviour: a fault a program causes
/// must kill the program, not the machine. The `AC` is the point of the first:
/// interrupt delivery does not clear it, so a program that sets it hands the
/// kernel a trap with SMAP already off unless the entry path clears it
/// (D-101).
///
/// The trailing jump is unreachable while the kernel retires its callers, and
/// it is there because a program whose last instruction falls forwards
/// executes whatever the page holds.
const CODE: [u8; 22] = [
    0x58, // pop rax
    0x48, 0x85, 0xC0, // test rax, rax
    0x74, 0x02, // jz +2
    0x0F, 0x0B, // ud2
    0x9C, // pushfq
    0x48, 0x81, 0x0C, 0x24, 0x00, 0x00, 0x04, 0x00, // or qword [rsp], 1 << 18
    0x9D, // popfq
    0xCD, 0x80, // int 0x80
    0xEB, 0xFE, // jmp .
];

/// `PT_LOAD`.
const PT_LOAD: u32 = 1;

/// Readable and executable, and deliberately not writable: W^X is not relaxed
/// for user memory (D-12).
const PF_READ_EXECUTE: u32 = 0b101;

/// Build the program image.
///
/// One loadable segment holding the code, at [`LOAD_ADDRESS`].
///
/// `p_align` is one, and deliberately: this kernel's loader *copies* into
/// fresh frames rather than mapping the file, so it needs no congruence
/// between file offset and virtual address. An earlier version of this comment
/// claimed the offset was chosen to hold that congruence, which it is not —
/// `p_offset` is 120 and `p_vaddr` is page-aligned (D-101). A loader that
/// mapped the image directly would need both the congruence and an alignment
/// that says so.
pub fn build() -> Vec<u8> {
    let mut image = vec![0u8; HEADER_BYTES + PROGRAM_HEADER_BYTES + CODE.len()];

    image[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    image[4] = 2; // ELFCLASS64
    image[5] = 1; // ELFDATA2LSB
    image[6] = 1; // EV_CURRENT
    image[16..18].copy_from_slice(&2u16.to_le_bytes()); // ET_EXEC
    image[18..20].copy_from_slice(&0x3Eu16.to_le_bytes()); // EM_X86_64
    image[20..24].copy_from_slice(&1u32.to_le_bytes()); // e_version
    image[24..32].copy_from_slice(&LOAD_ADDRESS.to_le_bytes()); // e_entry
    image[32..40].copy_from_slice(&(HEADER_BYTES as u64).to_le_bytes()); // e_phoff
    image[52..54].copy_from_slice(&(HEADER_BYTES as u16).to_le_bytes()); // e_ehsize
    image[54..56].copy_from_slice(&(PROGRAM_HEADER_BYTES as u16).to_le_bytes());
    image[56..58].copy_from_slice(&1u16.to_le_bytes()); // e_phnum

    let code_at = HEADER_BYTES + PROGRAM_HEADER_BYTES;
    let at = HEADER_BYTES;
    image[at..at + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
    image[at + 4..at + 8].copy_from_slice(&PF_READ_EXECUTE.to_le_bytes());
    image[at + 8..at + 16].copy_from_slice(&(code_at as u64).to_le_bytes()); // p_offset
    image[at + 16..at + 24].copy_from_slice(&LOAD_ADDRESS.to_le_bytes()); // p_vaddr
    image[at + 24..at + 32].copy_from_slice(&LOAD_ADDRESS.to_le_bytes()); // p_paddr
    image[at + 32..at + 40].copy_from_slice(&(CODE.len() as u64).to_le_bytes()); // p_filesz
    image[at + 40..at + 48].copy_from_slice(&(CODE.len() as u64).to_le_bytes()); // p_memsz
    image[at + 48..at + 56].copy_from_slice(&1u64.to_le_bytes()); // p_align

    image[code_at..code_at + CODE.len()].copy_from_slice(&CODE);
    image
}

#[cfg(test)]
mod tests {
    use super::*;
    use elf64::{Elf, SegmentKind};

    /// The image the builder writes is the image the kernel's reader sees.
    ///
    /// Parsed with `libs/elf64` rather than with a second copy of the layout
    /// here: a builder checked against its own understanding agrees with
    /// itself whatever it does, and the only interesting question is whether
    /// it agrees with the consumer.
    #[test]
    fn the_program_parses_as_the_kernel_will_read_it() {
        let image = build();
        let elf = Elf::parse(&image).expect("the built image is an elf64");

        assert_eq!(elf.entry(), LOAD_ADDRESS);
        assert_eq!(elf.segment_count(), 1);

        let segment = elf.segments().next().expect("one segment");
        assert_eq!(segment.kind(), SegmentKind::Load);
        assert_eq!(segment.address(), LOAD_ADDRESS);
        assert_eq!(segment.file_size(), CODE.len() as u64);
        assert_eq!(segment.memory_size(), CODE.len() as u64);
        assert!(segment.contents_fit(image.len()));
        assert!(!segment.is_oversized());
    }

    /// It is readable and executable, and never writable.
    ///
    /// A loader that mapped this writable would be mapping a page that is both
    /// writable and executable, which no mapping in this kernel may be.
    #[test]
    fn the_program_is_not_writable() {
        let image = build();
        let segment = Elf::parse(&image)
            .expect("an elf64")
            .segments()
            .next()
            .expect("one segment");

        assert!(segment.is_readable());
        assert!(segment.is_executable());
        assert!(!segment.is_writable());
        assert!(!segment.is_write_execute());
    }

    /// The entry point is inside the segment that is loaded.
    ///
    /// A header whose entry lands outside every `PT_LOAD` is well formed and
    /// unrunnable, which is the kind of thing a hand-built image gets wrong
    /// and a linker never does.
    #[test]
    fn the_entry_point_is_inside_a_loadable_segment() {
        let image = build();
        let elf = Elf::parse(&image).expect("an elf64");
        let entry = elf.entry();

        assert!(
            elf.segments()
                .filter(|segment| segment.kind() == SegmentKind::Load)
                .any(|segment| {
                    entry >= segment.address() && entry < segment.address() + segment.memory_size()
                }),
            "the entry point is outside every loadable segment"
        );
    }

    /// The code really is the bytes above, at the offset the header names.
    #[test]
    fn the_bytes_are_where_the_header_says() {
        let image = build();
        let segment = Elf::parse(&image)
            .expect("an elf64")
            .segments()
            .next()
            .expect("one segment");

        let at = segment.offset() as usize;
        let len = segment.file_size() as usize;
        assert_eq!(&image[at..at + len], &CODE);
    }
}
