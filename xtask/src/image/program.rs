//! The user program that ships in the image.
//!
//! A few hundred bytes of machine code inside a minimal ELF64, assembled here
//! rather than compiled. That is a deliberate stopping point: a real userland
//! needs a second crate, a second link address and a fourth pass in the gate's
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

/// The program: validate its entry block, then choose one of six behaviours.
///
/// ```text
///   59                       pop rcx                     ; entry ABI version
///   48 83 f9 01              cmp rcx, 1
///   75 ..                    jne die
///   58                       pop rax                     ; launch argument
///   5e                       pop rsi                     ; bootstrap handle
///   48 85 c0                 test rax, rax
///   78 ..                    js  leap                    ; kernel half: jump to it
///   48 83 f8 02              cmp rax, 2
///   75 ..                    jne ring-kind
///   b8 ad 0b ad 0b           mov eax, 0x0bad0bad         ; two: an unserved call
///   cd 80                    int 0x80
///   31 c0                    xor eax, eax
///   cd 80                    int 0x80
/// ring-kind:
///   ...                      zero selects Echo; three Duplicate; four Timer
/// ring:
///   9c                       pushfq
///   48 81 0c 24 00 00 04 00  or qword ptr [rsp], 0x40000 ; set AC, bit 18
///   9d                       popfq
///   48 bb ..                 mov rbx, RING               ; its own ring
///   c7 83 04 10 00 00 ..     mov dword [rbx+0x1004], tag
///   ...                      write Echo(argument), or
///                            Duplicate(bootstrap handle, READ)
///   c7 43 14 01 00 00 00     mov dword [rbx+20], 1       ; submission tail
///   b8 01 00 00 00           mov eax, 1                  ; ring enter
///   cd 80                    int 0x80
///   48 83 f8 01              cmp rax, 1                  ; exactly one served
///   75 ..                    jne die
///   8b 8b 04 11 00 00        mov ecx, [rbx+0x1104]       ; completion status
///   85 c9                    test ecx, ecx
///   75 ..                    jnz die
///   48 8b 8b 08 11 00 00     mov rcx, [rbx+0x1108]       ; completion value
///   ...                      require the echo, or a new nonzero handle
///   31 c0                    xor eax, eax                ; exit
///   cd 80                    int 0x80
/// leap:
///   ff e0                    jmp rax                     ; into whatever it was given
/// die:
///   0f 0b                    ud2
///   eb fe                    jmp .
/// ```
///
/// Six behaviours from one image, chosen by the versioned entry block the
/// kernel writes with `copy_to_user`: Echo, Duplicate, Timer plus RingWait, a
/// jump into the kernel half, an unserved call, and a deliberate fault.
/// Duplicate is the first to
/// consume the numeric bootstrap handle and proves that a trap-served
/// authority result reaches its prepaid arena slot without allocating
/// (D-102, D-182).
///
/// The `ud2` is the point of the fault route: a fault a program causes must
/// kill the program, not the machine. The `AC` is the point of the ring route:
/// interrupt delivery does not clear it, so a program that sets it hands the
/// kernel a trap with SMAP already off unless the entry path clears it (D-101).
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
    /// Bytes in one submission entry, from `kernel/src/queue.rs`.
    ///
    /// Thirty-two since slice 9 widened it for `Read`. This program writes one
    /// by hand, so its actual displacement operands are compared with the
    /// kernel's field map by a host test (D-173, D-181).
    const SUBMISSION_BYTES: u32 = 32;
    /// First completion entry: eight submissions after them.
    const COMPLETIONS_AT: u32 = SUBMISSIONS_AT + 8 * SUBMISSION_BYTES;
    /// `header::SUBMISSION_TAIL`.
    const TAIL_AT: u8 = 20;
    /// `WIZ_OP_ECHO`.
    const ECHO: u32 = 1;
    /// `WIZ_OP_DUPLICATE`.
    const DUPLICATE: u32 = 3;
    /// `WIZ_OP_TIMER`.
    const TIMER: u32 = 4;
    /// The call numbers, from `enum wiz_call`.
    const RING_ENTER: u32 = 1;
    const RING_WAIT: u32 = 3;

    let mut code = Vec::new();

    // The versioned entry block, and the six behaviours it chooses.
    code.extend_from_slice(&[0x59]); // pop rcx
    code.extend_from_slice(&[0x48, 0x83, 0xF9, 0x01]); // cmp rcx, 1
    let wrong_version = branch(&mut code, 0x75); // jne die
    code.extend_from_slice(&[0x58]); // pop rax
    code.extend_from_slice(&[0x5E]); // pop rsi
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
    let leaping = branch(&mut code, 0x78); // js leap
    let dispatching = branch(&mut code, 0xEB); // jmp dispatch
    let leap = code.len();
    patch(&mut code, leaping, leap);
    code.extend_from_slice(&[0xFF, 0xE0]); // jmp rax

    let dispatch = code.len();
    patch(&mut code, dispatching, dispatch);
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x02]); // cmp rax, 2
    let not_probing = branch(&mut code, 0x75); // jne ring kind

    // Argument two: ask for a call this kernel does not serve, and then exit
    // normally. Logging that refusal from trap context halted the machine
    // (D-122), so a program doing it is the check.
    code.extend_from_slice(&[0xB8]); // mov eax, imm32
    code.extend_from_slice(&UNSERVED_CALL.to_le_bytes());
    code.extend_from_slice(&[0xCD, 0x80]); // int 0x80
    code.extend_from_slice(&[0x31, 0xC0]); // xor eax, eax
    code.extend_from_slice(&[0xCD, 0x80]); // int 0x80
    code.extend_from_slice(&[0x0F, 0x0B, 0xEB, 0xFE]); // ud2; jmp .

    let choose_ring_kind = code.len();
    patch(&mut code, not_probing, choose_ring_kind);
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
    let not_echoing = branch(&mut code, 0x75); // jne duplicate selector
    code.extend_from_slice(&[0x31, 0xFF]); // xor edi, edi
    let echoing = branch(&mut code, 0xEB); // jmp ring

    let choose_non_echo = code.len();
    patch(&mut code, not_echoing, choose_non_echo);
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x03]); // cmp rax, 3
    let not_duplicating = branch(&mut code, 0x75); // jne timer selector
    code.extend_from_slice(&[0xBF]); // mov edi, 1
    code.extend_from_slice(&1u32.to_le_bytes());
    let duplicating = branch(&mut code, 0xEB); // jmp ring

    let choose_timer = code.len();
    patch(&mut code, not_duplicating, choose_timer);
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x04]); // cmp rax, 4
    let unknown_behaviour = branch(&mut code, 0x75); // jne die
    code.extend_from_slice(&[0xBF]); // mov edi, 2
    code.extend_from_slice(&2u32.to_le_bytes());
    let waiting = branch(&mut code, 0xEB); // jmp ring

    let early_die = code.len();
    patch(&mut code, wrong_version, early_die);
    patch(&mut code, unknown_behaviour, early_die);
    code.extend_from_slice(&[0x0F, 0x0B, 0xEB, 0xFE]); // ud2; jmp .

    // Set `AC`, which the entry path must clear.
    let ring = code.len();
    patch(&mut code, echoing, ring);
    patch(&mut code, duplicating, ring);
    patch(&mut code, waiting, ring);
    code.extend_from_slice(&[0x9C]); // pushfq
    code.extend_from_slice(&[0x48, 0x81, 0x0C, 0x24, 0x00, 0x00, 0x04, 0x00]);
    code.extend_from_slice(&[0x9D]); // popfq

    // rbx = the ring.
    code.extend_from_slice(&[0x48, 0xBB]);
    code.extend_from_slice(&RING.to_le_bytes());

    // Both submissions use the same nonzero token.
    code.extend_from_slice(&[0xC7, 0x83]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 4).to_le_bytes());
    code.extend_from_slice(&PROGRAM_TAG.to_le_bytes());

    code.extend_from_slice(&[0x85, 0xFF]); // test edi, edi
    let non_echo_submission = branch(&mut code, 0x75); // jnz non-echo

    // Echo one word back.
    code.extend_from_slice(&[0xC7, 0x83]);
    code.extend_from_slice(&SUBMISSIONS_AT.to_le_bytes());
    code.extend_from_slice(&ECHO.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xC7, 0x83]); // mov qword [rbx+disp32], imm32
    code.extend_from_slice(&(SUBMISSIONS_AT + 24).to_le_bytes());
    code.extend_from_slice(&PROGRAM_ARGUMENT.to_le_bytes());
    let echo_ready = branch(&mut code, 0xEB); // jmp publish

    let choose_submission = code.len();
    patch(&mut code, non_echo_submission, choose_submission);
    code.extend_from_slice(&[0x83, 0xFF, 0x01]); // cmp edi, 1
    let timer_submission = branch(&mut code, 0x75); // jne timer

    // Duplicate the bootstrap channel with only READ. Its handle comes from
    // the entry block rather than from a build-time constant.
    code.extend_from_slice(&[0xC7, 0x83]);
    code.extend_from_slice(&SUBMISSIONS_AT.to_le_bytes());
    code.extend_from_slice(&DUPLICATE.to_le_bytes());
    code.extend_from_slice(&[0x89, 0xB3]); // mov dword [rbx+disp32], esi
    code.extend_from_slice(&(SUBMISSIONS_AT + 8).to_le_bytes());
    code.extend_from_slice(&[0x48, 0xC7, 0x83]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 24).to_le_bytes());
    code.extend_from_slice(&DUPLICATE_RIGHTS.to_le_bytes());
    let duplicate_ready = branch(&mut code, 0xEB); // jmp publish

    // A short timer has to leave RingEnter unfinished and wake through
    // RingWait. Its terminal value is the nonzero monotonic deadline.
    let timer = code.len();
    patch(&mut code, timer_submission, timer);
    code.extend_from_slice(&[0xC7, 0x83]);
    code.extend_from_slice(&SUBMISSIONS_AT.to_le_bytes());
    code.extend_from_slice(&TIMER.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xC7, 0x83]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 24).to_le_bytes());
    code.extend_from_slice(&2_000_000u32.to_le_bytes());

    // Publish it.
    let publish = code.len();
    patch(&mut code, echo_ready, publish);
    patch(&mut code, duplicate_ready, publish);
    code.extend_from_slice(&[0xC7, 0x43, TAIL_AT]); // mov dword [rbx+20], imm32
    code.extend_from_slice(&1u32.to_le_bytes());

    // Ring the doorbell.
    code.extend_from_slice(&[0xB8]); // mov eax, imm32
    code.extend_from_slice(&RING_ENTER.to_le_bytes());
    code.extend_from_slice(&[0xCD, 0x80]); // int 0x80

    // Timer admission completes nothing; RingWait must block and report it.
    code.extend_from_slice(&[0x83, 0xFF, 0x02]); // cmp edi, 2
    let immediate = branch(&mut code, 0x75); // jne immediate answer
    code.extend_from_slice(&[0x48, 0x85, 0xC0]); // test rax, rax
    let completed_early = branch(&mut code, 0x75); // jne die
    code.extend_from_slice(&[0xB8]); // mov eax, imm32
    code.extend_from_slice(&RING_WAIT.to_le_bytes());
    code.extend_from_slice(&[0xCD, 0x80]); // int 0x80

    // Exactly one served, or die.
    let expect_one = code.len();
    patch(&mut code, immediate, expect_one);
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x01]); // cmp rax, 1
    let served = branch(&mut code, 0x75); // jne die

    // With a clean status, or die.
    code.extend_from_slice(&[0x8B, 0x8B]); // mov ecx, [rbx+disp32]
    code.extend_from_slice(&(COMPLETIONS_AT + 4).to_le_bytes());
    code.extend_from_slice(&[0x85, 0xC9]); // test ecx, ecx
    let clean = branch(&mut code, 0x75); // jnz die

    // The value came back and is the kind this behaviour promised.
    code.extend_from_slice(&[0x48, 0x8B, 0x8B]); // mov rcx, [rbx+disp32]
    code.extend_from_slice(&(COMPLETIONS_AT + 8).to_le_bytes());
    code.extend_from_slice(&[0x85, 0xFF]); // test edi, edi
    let non_echo_answer = branch(&mut code, 0x75); // jnz non-echo answer
    code.extend_from_slice(&[0x48, 0xC7, 0xC2]); // mov rdx, imm32
    code.extend_from_slice(&PROGRAM_ARGUMENT.to_le_bytes());
    code.extend_from_slice(&[0x48, 0x39, 0xD1]); // cmp rcx, rdx
    let echoed = branch(&mut code, 0x75); // jne die
    let answered = branch(&mut code, 0xEB); // jmp exit

    let choose_answer = code.len();
    patch(&mut code, non_echo_answer, choose_answer);
    code.extend_from_slice(&[0x83, 0xFF, 0x02]); // cmp edi, 2
    let timer_answer = branch(&mut code, 0x74); // je timer answer
    code.extend_from_slice(&[0x48, 0x85, 0xC9]); // test rcx, rcx
    let empty_handle = branch(&mut code, 0x74); // jz die
    code.extend_from_slice(&[0x39, 0xF1]); // cmp ecx, esi
    let repeated_handle = branch(&mut code, 0x74); // je die
    let duplicated = branch(&mut code, 0xEB); // jmp exit

    let timer_value = code.len();
    patch(&mut code, timer_answer, timer_value);
    code.extend_from_slice(&[0x48, 0x85, 0xC9]); // test rcx, rcx
    let empty_deadline = branch(&mut code, 0x74); // jz die

    // Exit.
    let exit = code.len();
    patch(&mut code, answered, exit);
    patch(&mut code, duplicated, exit);
    code.extend_from_slice(&[0x31, 0xC0]); // xor eax, eax
    code.extend_from_slice(&[0xCD, 0x80]); // int 0x80

    // Everything that went wrong lands here.
    let die = code.len();
    code.extend_from_slice(&[0x0F, 0x0B]); // ud2
    code.extend_from_slice(&[0xEB, 0xFE]); // jmp .

    for site in [
        completed_early,
        served,
        clean,
        echoed,
        empty_handle,
        repeated_handle,
        empty_deadline,
    ] {
        patch(&mut code, site, die);
    }
    code
}

/// The tag the program puts on its submission, and gets back.
const PROGRAM_TAG: u32 = 0x0000_1234;

/// The word it asks the kernel to echo.
const PROGRAM_ARGUMENT: u32 = 0x00C0_FFEE;

/// Rights the ring-3 Duplicate asks to retain: Channel READ and nothing else.
const DUPLICATE_RIGHTS: u32 = 1 << 8;

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
    let displacement = i8::try_from(distance)
        .unwrap_or_else(|_| panic!("jump from {from} to {target} does not fit in one byte"));
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

        // The entry version is consumed before the argument and handle.
        assert_eq!(
            &code[..12],
            &[
                0x59, 0x48, 0x83, 0xF9, 0x01, 0x75, code[6], 0x58, 0x5E, 0x48, 0x85, 0xC0,
            ],
            "pop rcx; cmp rcx, 1; jne die; pop rax; pop rsi; test rax, rax"
        );
        // Six behaviours: Echo, Duplicate, Timer+Wait, an unserved call, a
        // deliberate fault, and a jump to a supplied kernel-half address.
        assert_eq!(code[12], 0x78, "js leap");
        assert!(
            code.windows(4)
                .any(|bytes| bytes == [0x48, 0x83, 0xF8, 0x02]),
            "cmp rax, 2"
        );
        assert!(
            code.windows(4)
                .any(|bytes| bytes == [0x48, 0x83, 0xF8, 0x03]),
            "cmp rax, 3"
        );
        assert!(
            code.windows(4)
                .any(|bytes| bytes == [0x48, 0x83, 0xF8, 0x04]),
            "cmp rax, 4"
        );
        // And the leap itself, which is what makes the argument an address.
        assert!(code.windows(2).any(|pair| pair == [0xFF, 0xE0]), "jmp rax");

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
        assert!(
            code.windows(10)
                .any(|bytes| bytes == [0xC7, 0x83, 0x00, 0x10, 0, 0, 1, 0, 0, 0]),
            "mov dword [rbx+0x1000], 1"
        );
        assert!(
            code.windows(10)
                .any(|bytes| bytes == [0xC7, 0x83, 0x00, 0x10, 0, 0, 3, 0, 0, 0]),
            "mov dword [rbx+0x1000], 3"
        );
        assert!(
            code.windows(10)
                .any(|bytes| bytes == [0xC7, 0x83, 0x00, 0x10, 0, 0, 4, 0, 0, 0]),
            "mov dword [rbx+0x1000], 4"
        );
        assert!(
            code.windows(10)
                .any(|bytes| { bytes == [0xC7, 0x83, 0x04, 0x10, 0, 0, 0x34, 0x12, 0, 0] }),
            "mov dword [rbx+0x1004], tag"
        );
        assert!(
            code.windows(11).any(|bytes| {
                bytes == [0x48, 0xC7, 0x83, 0x18, 0x10, 0, 0, 0xEE, 0xFF, 0xC0, 0]
            }),
            "mov qword [rbx+0x1018], argument"
        );
        assert!(
            code.windows(6)
                .any(|bytes| bytes == [0x89, 0xB3, 0x08, 0x10, 0, 0]),
            "mov dword [rbx+0x1008], esi"
        );
        assert!(
            code.windows(11)
                .any(|bytes| { bytes == [0x48, 0xC7, 0x83, 0x18, 0x10, 0, 0, 0, 1, 0, 0] }),
            "mov qword [rbx+0x1018], READ"
        );
        assert!(
            code.windows(7)
                .any(|bytes| bytes == [0x48, 0x8B, 0x8B, 0x08, 0x11, 0, 0]),
            "mov rcx, [rbx+0x1108]"
        );
        assert!(
            code.windows(6)
                .any(|bytes| bytes == [0x8B, 0x8B, 0x04, 0x11, 0, 0]),
            "mov ecx, [rbx+0x1104]"
        );

        // Five `int 0x80`s: the unserved probe and its exit, then RingEnter,
        // the timer-only RingWait, and the common exit.
        let calls: Vec<usize> = code
            .windows(2)
            .enumerate()
            .filter(|(_, pair)| *pair == [0xCD, 0x80])
            .map(|(at, _)| at)
            .collect();
        assert_eq!(calls.len(), 5, "three calls and two exits");
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
        assert_eq!(
            &code[calls[3] - 5..calls[3]],
            &[0xB8, 3, 0, 0, 0],
            "mov eax, 3"
        );
        assert_eq!(&code[calls[4] - 2..calls[4]], &[0x31, 0xC0], "xor eax, eax");

        // Every terminal validation branch lands on the final `ud2`.
        let die = code.len() - 4;
        assert_eq!(&code[die..], &[0x0F, 0x0B, 0xEB, 0xFE], "ud2; jmp .");
        let failures = (0..code.len() - 1)
            .filter(|site| matches!(code[*site], 0x74 | 0x75))
            .filter(|site| *site as i64 + 2 + i64::from(code[*site + 1] as i8) == die as i64)
            .count();
        assert_eq!(
            failures, 7,
            "seven terminal validations reach the final ud2"
        );
    }
}
