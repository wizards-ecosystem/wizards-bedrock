use super::*;
use elf64::{Elf, SegmentKind};

#[test]
fn the_program_parses_as_the_kernel_will_read_it() {
    let image = build().expect("building the test program");
    let code = code().expect("building the test program code");
    let elf = Elf::parse(&image).expect("the built image is an elf64");

    assert_eq!(elf.entry(), LOAD_ADDRESS);
    assert_eq!(elf.segment_count(), 1);

    let segment = elf.segments().next().expect("one segment");
    assert_eq!(segment.kind(), SegmentKind::Load);
    assert_eq!(segment.address(), LOAD_ADDRESS);
    assert_eq!(segment.file_size(), code.len() as u64);
    assert_eq!(segment.memory_size(), code.len() as u64);
    assert!(segment.contents_fit(image.len()));
    assert!(!segment.is_oversized());
}

#[test]
fn the_program_is_not_writable() {
    let image = build().expect("building the test program");
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

#[test]
fn the_entry_point_is_inside_a_loadable_segment() {
    let image = build().expect("building the test program");
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

#[test]
fn the_bytes_are_where_the_header_says() {
    let image = build().expect("building the test program");
    let code = code().expect("building the test program code");
    let segment = Elf::parse(&image)
        .expect("an elf64")
        .segments()
        .next()
        .expect("one segment");

    let at = segment.offset() as usize;
    let len = segment.file_size() as usize;
    assert_eq!(&image[at..at + len], code.as_slice());
}

#[test]
fn the_emitted_program_has_required_routes() {
    let code = code().expect("building the test program code");

    assert_eq!(
        &code[..18],
        &[
            0x59, 0x48, 0x83, 0xF9, 0x01, 0x75, 0x02, 0xEB, 0x04, 0x0F, 0x0B, 0xEB, 0xFE, 0x58,
            0x5E, 0x48, 0x85, 0xC0,
        ],
        "pop rcx; cmp rcx, 1; jne fail; jmp payload; ud2; loop; pop rax; pop rsi; test rax, rax"
    );
    assert_eq!(code[18], 0x78, "js leap");
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
    assert!(
        code.windows(4)
            .any(|bytes| bytes == [0x48, 0x83, 0xF8, 0x05]),
        "cmp rax, 5"
    );
    assert!(
        code.windows(4)
            .any(|bytes| bytes == [0x48, 0x83, 0xF8, INVALID_STACK_BEHAVIOUR]),
        "cmp rax, invalid-stack behaviour"
    );
    assert!(
        code.windows(4)
            .any(|bytes| bytes == [0x48, 0x83, 0xF8, INVALID_FLAGS_BEHAVIOUR]),
        "cmp rax, invalid-flags behaviour"
    );
    assert!(code.windows(2).any(|pair| pair == [0xFF, 0xE0]), "jmp rax");

    // Setting `AC`, which the entry path must clear. Searched for, not
    // indexed, so a behaviour added above moves the assertion with it.
    let flags = code
        .windows(10)
        .position(|window| window == [0x9C, 0x48, 0x81, 0x0C, 0x24, 0x00, 0x00, 0x04, 0x00, 0x9D])
        .expect("pushfq; or qword [rsp], 0x40000; popfq");

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
        code.windows(11)
            .any(|bytes| { bytes == [0x48, 0xC7, 0x83, 0x18, 0x10, 0, 0, 0xEE, 0xFF, 0xC0, 0] }),
        "mov qword [rbx+0x1018], argument"
    );
    assert!(
        code.windows(6)
            .any(|bytes| bytes == [0x89, 0xB3, 0x08, 0x10, 0, 0]),
        "mov dword [rbx+0x1008], esi"
    );
    assert!(
        code.windows(11)
            .any(|bytes| { bytes == [0x48, 0xC7, 0x83, 0x18, 0x10, 0, 0, 8, 0, 0, 0] }),
        "mov qword [rbx+0x1018], INSPECT"
    );
    assert!(
        code.windows(6)
            .any(|bytes| bytes == [0x41, 0xB8, 1, 0, 0, 0]),
        "mov r8d, measured"
    );
    assert_eq!(
        code.windows(5)
            .filter(|bytes| *bytes == [0x0F, 0xAE, 0xE8, 0x0F, 0x31])
            .count(),
        2,
        "two fenced timestamp reads bracket RingEnter"
    );
    assert!(
        code.windows(6)
            .any(|bytes| bytes == [0x89, 0x83, 0x10, 0x10, 0, 0]),
        "mov dword [rbx+0x1010], eax"
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

    let calls: Vec<usize> = code
        .windows(2)
        .enumerate()
        .filter(|(_, pair)| *pair == [0xCD, 0x80])
        .map(|(at, _)| at)
        .collect();
    assert_eq!(
        calls.len(),
        7,
        "two hostile returns, three calls and two exits"
    );
    assert_eq!(
        &code[calls[0] - 10..calls[0] - 8],
        &[0x48, 0xBC],
        "mov rsp, imm64 before the hostile call"
    );
    assert_eq!(
        u64::from_le_bytes(
            code[calls[0] - 8..calls[0]]
                .try_into()
                .expect("eight-byte stack immediate")
        ),
        NONCANONICAL_STACK,
        "the hostile call carries the canonical-hole boundary in rsp"
    );
    assert_eq!(
        &code[calls[1] - 15..calls[1] - 6],
        &[0x9C, 0x48, 0x81, 0x0C, 0x24, 0x00, 0x40, 0x00, 0x00],
        "pushfq; or qword [rsp], NT"
    );
    assert_eq!(code[calls[1] - 6], 0x9D, "popfq");
    assert_eq!(
        &code[calls[2] - 5..calls[2]],
        &[0xB8, 0xAD, 0x0B, 0xAD, 0x0B],
        "mov eax, a call the kernel does not serve"
    );
    assert_eq!(&code[calls[3] - 2..calls[3]], &[0x31, 0xC0], "xor eax, eax");
    assert_eq!(
        &code[calls[4] - 5..calls[4]],
        &[0xB8, 1, 0, 0, 0],
        "mov eax, 1"
    );
    assert_eq!(
        &code[calls[5] - 5..calls[5]],
        &[0xB8, 3, 0, 0, 0],
        "mov eax, 3"
    );
    assert_eq!(&code[calls[6] - 2..calls[6]], &[0x31, 0xC0], "xor eax, eax");

    let die = code.len() - 4;
    assert_eq!(&code[die..], &[0x0F, 0x0B, 0xEB, 0xFE], "ud2; jmp .");
    let failures = (0..code.len() - 1)
        .filter(|site| matches!(code[*site], 0x74 | 0x75))
        .filter(|site| *site as i64 + 2 + i64::from(code[*site + 1] as i8) == die as i64)
        .count();
    assert_eq!(
        failures, 8,
        "eight terminal validations reach the final ud2"
    );
}

#[test]
fn the_ring_client_orders_shared_memory() {
    let code = code().expect("building the test program code");
    let find = |instruction: &[u8]| {
        code.windows(instruction.len())
            .position(|bytes| bytes == instruction)
            .unwrap_or_else(|| panic!("missing instruction {instruction:02x?}"))
    };

    let payload = find(&[0xC7, 0x83, 0x04, 0x10, 0, 0, 0x34, 0x12, 0, 0]);
    let submission_tail = find(&[0xC7, 0x43, 0x14, 1, 0, 0, 0]);
    let completion_tail = find(&[0x8B, 0x43, 0x1C, 0x83, 0xF8, 1]);
    let completion_status = find(&[0x8B, 0x8B, 0x04, 0x11, 0, 0]);
    let completion_value = find(&[0x48, 0x8B, 0x8B, 0x08, 0x11, 0, 0]);
    let completion_head = find(&[0xC7, 0x43, 0x18, 1, 0, 0, 0]);

    assert!(
        payload < submission_tail
            && submission_tail < completion_tail
            && completion_tail < completion_status
            && completion_status < completion_value
            && completion_value < completion_head,
        "payload, SQ release, CQ acquire, completion copy and CQ release are out of order"
    );
}

#[test]
fn branch_patching_refuses_an_invalid_site() {
    let mut code = [0_u8];

    let error = patch(&mut code, 1, 0).expect_err("an out-of-bounds patch must fail");

    assert!(error.to_string().contains("outside its 1-byte program"));
}

#[test]
fn branch_patching_enforces_the_signed_byte_extent() {
    let mut forward = [0_u8];
    patch(&mut forward, 0, 128).expect("the positive rel8 boundary");
    assert_eq!(forward[0], 0x7f);
    assert!(patch(&mut forward, 0, 129).is_err());

    let mut backward = [0_u8; 130];
    patch(&mut backward, 129, 2).expect("the negative rel8 boundary");
    assert_eq!(backward[129], 0x80);
    assert!(patch(&mut backward, 129, 1).is_err());
}
