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

/// The program: read one word of argument, then either fault deliberately or
/// drive its own ring and exit.
///
/// ```text
///   58                       pop rax                     ; the kernel's word
///   48 85 c0                 test rax, rax
///   74 02                    jz  behave                  ; zero: behave
///   0f 0b                    ud2                         ; anything else: die
/// behave:
///   9c                       pushfq
///   48 81 0c 24 00 00 04 00  or qword ptr [rsp], 0x40000 ; set AC, bit 18
///   9d                       popfq
///   48 bb ..                 mov rbx, RING               ; its own ring
///   c7 83 00 10 00 00 ..     mov dword [rbx+0x1000], 1   ; opcode: echo
///   c7 83 04 10 00 00 ..     mov dword [rbx+0x1004], tag
///   48 c7 83 08 10 00 00 ..  mov qword [rbx+0x1008], arg
///   c7 43 14 01 00 00 00     mov dword [rbx+20], 1       ; submission tail
///   b8 01 00 00 00           mov eax, 1                  ; ring enter
///   cd 80                    int 0x80
///   48 83 f8 01              cmp rax, 1                  ; exactly one served
///   75 ..                    jne die
///   48 8b 8b 88 10 00 00     mov rcx, [rbx+0x1088]       ; completion value
///   48 c7 c2 ..              mov rdx, arg
///   48 39 d1                 cmp rcx, rdx                ; it echoed back
///   75 ..                    jne die
///   8b 8b 84 10 00 00        mov ecx, [rbx+0x1084]       ; completion status
///   85 c9                    test ecx, ecx
///   75 ..                    jnz die
///   31 c0                    xor eax, eax                ; exit
///   cd 80                    int 0x80
/// die:
///   0f 0b                    ud2
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
/// **The ring work is the point of everything after that** (D-116). Until it
/// existed the program crossed once and was retired, so the whole async ABI —
/// the shared pages, the admission, the terminal completion — was reachable
/// only from kernel checks. Here a *program* posts an operation, rings the
/// doorbell, and reads its own answer back out of a page it shares with the
/// kernel. Every refusal on the way is a `ud2`, so a wrong answer kills the
/// program and the boot check sees a fault instead of an exit.
///
/// Emitted rather than transcribed: the jump displacements are patched from
/// the labels, so adding an instruction cannot silently retarget a branch —
/// which is the failure mode a hand-assembled listing has and does not
/// announce.
fn code() -> Vec<u8> {
    /// Where the kernel maps a program's ring. Must match `user::RING`.
    const RING: u64 = 0x0000_2000_0000_0000;
    /// First submission entry, from the ABI's `WIZ_RING_ENTRIES_AT`.
    const SUBMISSIONS_AT: u32 = 4096;
    /// First completion entry: eight submissions of sixteen bytes after them.
    const COMPLETIONS_AT: u32 = SUBMISSIONS_AT + 8 * 16;
    /// `header::SUBMISSION_TAIL`.
    const TAIL_AT: u8 = 20;
    /// `WIZ_OP_ECHO`.
    const ECHO: u32 = 1;
    /// The call numbers, from `enum wiz_call`.
    const RING_ENTER: u32 = 1;

    let mut code = Vec::new();

    // The argument word, and the three behaviours it chooses.
    code.extend_from_slice(&[0x58]); // pop rax
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
    // Patched from the label like every other branch. Hand-computing this one
    // put it two bytes past `behave` and into the middle of the probe's exit,
    // which assembled, ran, and did something else — the exact failure the
    // patch machinery exists to prevent, made by the person who wrote it.
    let behaving = branch(&mut code, 0x74); // jz behave
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x02]); // cmp rax, 2
    let not_probing = branch(&mut code, 0x75); // jne die

    // Argument two: ask for a call this kernel does not serve, and then exit
    // normally. Logging that refusal from trap context halted the machine
    // (D-122), so a program doing it is the check.
    code.extend_from_slice(&[0xB8]); // mov eax, imm32
    code.extend_from_slice(&UNSERVED_CALL.to_le_bytes());
    code.extend_from_slice(&[0xCD, 0x80]); // int 0x80
    code.extend_from_slice(&[0x31, 0xC0]); // xor eax, eax
    code.extend_from_slice(&[0xCD, 0x80]); // int 0x80

    // Set `AC`, which the entry path must clear.
    let behave = code.len();
    patch(&mut code, behaving, behave);
    code.extend_from_slice(&[0x9C]); // pushfq
    code.extend_from_slice(&[0x48, 0x81, 0x0C, 0x24, 0x00, 0x00, 0x04, 0x00]);
    code.extend_from_slice(&[0x9D]); // popfq

    // rbx = the ring.
    code.extend_from_slice(&[0x48, 0xBB]);
    code.extend_from_slice(&RING.to_le_bytes());

    // One submission: echo, tagged, with an argument to get back.
    code.extend_from_slice(&[0xC7, 0x83]); // mov dword [rbx+disp32], imm32
    code.extend_from_slice(&SUBMISSIONS_AT.to_le_bytes());
    code.extend_from_slice(&ECHO.to_le_bytes());

    code.extend_from_slice(&[0xC7, 0x83]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 4).to_le_bytes());
    code.extend_from_slice(&PROGRAM_TAG.to_le_bytes());

    code.extend_from_slice(&[0x48, 0xC7, 0x83]); // mov qword [rbx+disp32], imm32
    code.extend_from_slice(&(SUBMISSIONS_AT + 8).to_le_bytes());
    code.extend_from_slice(&PROGRAM_ARGUMENT.to_le_bytes());

    // Publish it.
    code.extend_from_slice(&[0xC7, 0x43, TAIL_AT]); // mov dword [rbx+20], imm32
    code.extend_from_slice(&1u32.to_le_bytes());

    // Ring the doorbell.
    code.extend_from_slice(&[0xB8]); // mov eax, imm32
    code.extend_from_slice(&RING_ENTER.to_le_bytes());
    code.extend_from_slice(&[0xCD, 0x80]); // int 0x80

    // Exactly one served, or die.
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x01]); // cmp rax, 1
    let served = branch(&mut code, 0x75); // jne die

    // The value came back, or die.
    code.extend_from_slice(&[0x48, 0x8B, 0x8B]); // mov rcx, [rbx+disp32]
    code.extend_from_slice(&(COMPLETIONS_AT + 8).to_le_bytes());
    code.extend_from_slice(&[0x48, 0xC7, 0xC2]); // mov rdx, imm32
    code.extend_from_slice(&PROGRAM_ARGUMENT.to_le_bytes());
    code.extend_from_slice(&[0x48, 0x39, 0xD1]); // cmp rcx, rdx
    let echoed = branch(&mut code, 0x75); // jne die

    // With a clean status, or die.
    code.extend_from_slice(&[0x8B, 0x8B]); // mov ecx, [rbx+disp32]
    code.extend_from_slice(&(COMPLETIONS_AT + 4).to_le_bytes());
    code.extend_from_slice(&[0x85, 0xC9]); // test ecx, ecx
    let clean = branch(&mut code, 0x75); // jnz die

    // Exit.
    code.extend_from_slice(&[0x31, 0xC0]); // xor eax, eax
    code.extend_from_slice(&[0xCD, 0x80]); // int 0x80

    // Everything that went wrong lands here.
    let die = code.len();
    code.extend_from_slice(&[0x0F, 0x0B]); // ud2
    code.extend_from_slice(&[0xEB, 0xFE]); // jmp .

    for site in [not_probing, served, echoed, clean] {
        patch(&mut code, site, die);
    }
    code
}

/// The tag the program puts on its submission, and gets back.
const PROGRAM_TAG: u32 = 0x0000_1234;

/// The word it asks the kernel to echo.
const PROGRAM_ARGUMENT: u32 = 0x00C0_FFEE;

/// A call number this kernel does not serve.
///
/// Far above anything `enum wiz_call` names, so adding a call cannot turn this
/// into a real one by accident.
const UNSERVED_CALL: u32 = 0x0BAD_0BAD;

/// Emit a two-byte conditional jump with a placeholder, and report where its
/// displacement byte is.
fn branch(code: &mut Vec<u8>, opcode: u8) -> usize {
    code.push(opcode);
    code.push(0);
    code.len() - 1
}

/// Fill in a displacement, now that the target is known.
///
/// Panics on a jump too far to encode, which is a build-time failure and the
/// right one: a truncated displacement is a branch into the middle of an
/// instruction, and the program would run and do something else.
fn patch(code: &mut [u8], site: usize, target: usize) {
    let from = site + 1;
    let distance = i64::try_from(target).expect("a small program")
        - i64::try_from(from).expect("a small program");
    let displacement = i8::try_from(distance).expect("a jump inside one byte");
    code[site] = displacement as u8;
}

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
    let code = code();
    let mut image = vec![0u8; HEADER_BYTES + PROGRAM_HEADER_BYTES + code.len()];

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
    image[at + 32..at + 40].copy_from_slice(&(code.len() as u64).to_le_bytes()); // p_filesz
    image[at + 40..at + 48].copy_from_slice(&(code.len() as u64).to_le_bytes()); // p_memsz
    image[at + 48..at + 56].copy_from_slice(&1u64.to_le_bytes()); // p_align

    image[code_at..code_at + code.len()].copy_from_slice(&code);
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
        assert_eq!(segment.file_size(), code().len() as u64);
        assert_eq!(segment.memory_size(), code().len() as u64);
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
        assert_eq!(&image[at..at + len], code().as_slice());
    }

    /// The emitted instructions are the ones the listing says.
    ///
    /// A generated program can be wrong in a way a transcribed one cannot: an
    /// instruction added in the middle shifts every later one, and a
    /// displacement computed from the wrong label is a jump into the middle of
    /// an instruction that still *runs*. So the bytes are pinned here against
    /// the doc comment above (D-116).
    #[test]
    fn the_emitted_program_is_the_listing() {
        let code = code();

        // The first four are what the kernel reads back across the boundary,
        // and `user::PROGRAM` transcribes them independently.
        assert_eq!(
            &code[..4],
            &[0x58, 0x48, 0x85, 0xC0],
            "pop rax; test rax, rax"
        );
        // Three behaviours, chosen by the argument: zero behaves, two asks for
        // a call this kernel does not serve, anything else faults.
        assert_eq!(code[4], 0x74, "jz behave");
        assert_eq!(&code[6..10], &[0x48, 0x83, 0xF8, 0x02], "cmp rax, 2");
        assert_eq!(code[10], 0x75, "jne die");

        // Setting `AC`, which the entry path must clear. Searched for rather
        // than indexed, so adding a behaviour above it moves the assertion
        // with the code instead of silently onto different bytes.
        let flags = code
            .windows(10)
            .position(|w| w == [0x9C, 0x48, 0x81, 0x0C, 0x24, 0x00, 0x00, 0x04, 0x00, 0x9D])
            .expect("pushfq; or qword [rsp], 0x40000; popfq");

        // The ring's address, which must be `user::RING`.
        let ring = flags + 10;
        assert_eq!(&code[ring..ring + 2], &[0x48, 0xBB], "mov rbx, imm64");
        assert_eq!(
            u64::from_le_bytes(code[ring + 2..ring + 10].try_into().expect("eight bytes")),
            0x0000_2000_0000_0000,
            "the program looks for its ring where the kernel maps it"
        );

        // Four `int 0x80`s: the unserved probe and its exit, then the doorbell
        // and its exit.
        let calls: Vec<usize> = code
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| *pair == [0xCD, 0x80])
            .map(|(at, _)| at)
            .collect();
        assert_eq!(calls.len(), 4, "two calls and two exits");
        assert_eq!(
            &code[calls[0] - 5..calls[0]],
            &[0xB8, 0xAD, 0x0B, 0xAD, 0x0B],
            "mov eax, a call the kernel does not serve"
        );
        assert_eq!(&code[calls[1] - 2..calls[1]], &[0x31, 0xC0], "xor eax, eax");
        assert_eq!(
            &code[calls[2] - 5..calls[2]],
            &[0xB8, 1, 0, 0, 0],
            "mov eax, 1"
        );
        assert_eq!(&code[calls[3] - 2..calls[3]], &[0x31, 0xC0], "xor eax, eax");

        // Every branch lands on the `ud2`, and none in the middle of an
        // instruction: the displacements are patched from one label, so if any
        // is wrong they all point somewhere that is not it.
        let die = code.len() - 4;
        assert_eq!(&code[die..], &[0x0F, 0x0B, 0xEB, 0xFE], "ud2; jmp .");
        // Scanned from the start rather than from an offset: the first branch
        // now precedes the flags block, and no immediate this program emits
        // contains 0x75.
        let branches: Vec<usize> = code
            .iter()
            .enumerate()
            .filter(|(at, byte)| **byte == 0x75 && *at + 1 < code.len())
            .map(|(at, _)| at)
            .collect();
        assert_eq!(branches.len(), 4, "four ways to fail");
        for site in branches {
            let displacement = i64::from(code[site + 1] as i8);
            let target = site as i64 + 2 + displacement;
            assert_eq!(target, die as i64, "a branch at {site} misses the ud2");
        }

        // And the one branch that is not a way to fail lands on the flags
        // block. Hand-computing it put it two bytes past, in the middle of an
        // instruction, where it assembled and ran and did something else.
        let behaving = i64::from(code[5] as i8);
        assert_eq!(
            4 + 2 + behaving,
            flags as i64,
            "the jz misses the start of the behaving path"
        );
    }
}
