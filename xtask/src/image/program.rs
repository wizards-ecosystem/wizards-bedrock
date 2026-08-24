//! The user program that ships in the image.
//!
//! Four bytes of machine code inside a minimal ELF64, assembled here rather
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

/// `int 0x80`, then a jump to itself.
///
/// The second instruction is unreachable while the kernel retires the caller,
/// and it is there because a program whose last instruction falls forwards
/// executes whatever the page holds.
const CODE: [u8; 4] = [0xCD, 0x80, 0xEB, 0xFE];

/// `PT_LOAD`.
const PT_LOAD: u32 = 1;

/// Readable and executable, and deliberately not writable: W^X is not relaxed
/// for user memory (D-12).
const PF_READ_EXECUTE: u32 = 0b101;

/// Build the program image.
///
/// One loadable segment holding the code, at [`LOAD_ADDRESS`], with the file
/// offset chosen so the segment's bytes are congruent to its virtual address
/// modulo a page — which is what any loader that maps rather than copies
/// requires, and what a hand-built ELF gets wrong first.
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

    /// The code really is the four bytes, at the offset the header names.
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
