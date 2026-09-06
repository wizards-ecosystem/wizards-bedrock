//! Builds the fixed ELF64 ring-3 boundary witness.
//!
//! Qualification decodes the emitted instructions independently, and the
//! kernel's ELF parser checks the container (D-100).

use anyhow::{Context, Result};
use elf64::{HEADER_BYTES, PROGRAM_HEADER_BYTES};

/// Where the program is linked, and where the kernel maps it.
pub const LOAD_ADDRESS: u64 = 0x0000_4000_0000_0000;

/// Build the ring-3 witness used by the boot checks.
///
/// Every wrong answer reaches `ud2`, making disagreement observable at boot.
fn code() -> Result<Vec<u8>> {
    const RING: u64 = 0x0000_2000_0000_0000;
    const SUBMISSIONS_AT: u32 = 4096;
    const SUBMISSION_BYTES: u32 = 32;
    const COMPLETIONS_AT: u32 = SUBMISSIONS_AT + 8 * SUBMISSION_BYTES;
    const SUBMISSION_TAIL_AT: u8 = 20;
    const COMPLETION_HEAD_AT: u8 = 24;
    const COMPLETION_TAIL_AT: u8 = 28;
    const ECHO: u32 = 1;
    const DUPLICATE: u32 = 3;
    const TIMER: u32 = 4;
    const RING_ENTER: u32 = 1;
    const RING_WAIT: u32 = 3;

    let mut code = Vec::new();

    code.extend_from_slice(&[0x59]);
    code.extend_from_slice(&[0x48, 0x83, 0xF9, 0x01]);
    let wrong_version = branch(&mut code, 0x75);
    code.extend_from_slice(&[0x58]);
    code.extend_from_slice(&[0x5E]);
    code.extend_from_slice(&[0x48, 0x85, 0xC0]);
    let leaping = branch(&mut code, 0x78);
    let dispatching = branch(&mut code, 0xEB);
    let leap = code.len();
    patch(&mut code, leaping, leap)?;
    code.extend_from_slice(&[0xFF, 0xE0]);

    let dispatch = code.len();
    patch(&mut code, dispatching, dispatch)?;
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x02]);
    let not_probing = branch(&mut code, 0x75);

    // An unsupported call proves trap-side refusal remains nonblocking (D-122).
    code.extend_from_slice(&[0xB8]);
    code.extend_from_slice(&UNSERVED_CALL.to_le_bytes());
    code.extend_from_slice(&[0xCD, 0x80]);
    code.extend_from_slice(&[0x31, 0xC0]);
    code.extend_from_slice(&[0xCD, 0x80]);
    code.extend_from_slice(&[0x0F, 0x0B, 0xEB, 0xFE]);

    let choose_ring_kind = code.len();
    patch(&mut code, not_probing, choose_ring_kind)?;
    code.extend_from_slice(&[0x45, 0x31, 0xC0]);
    code.extend_from_slice(&[0x48, 0x85, 0xC0]);
    let not_echoing = branch(&mut code, 0x75);
    code.extend_from_slice(&[0x31, 0xFF]);
    let echoing = branch(&mut code, 0xEB);

    let choose_non_echo = code.len();
    patch(&mut code, not_echoing, choose_non_echo)?;
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x03]);
    let not_duplicating = branch(&mut code, 0x75);
    code.extend_from_slice(&[0xBF]);
    code.extend_from_slice(&1u32.to_le_bytes());
    let duplicating = branch(&mut code, 0xEB);

    let choose_timer = code.len();
    patch(&mut code, not_duplicating, choose_timer)?;
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x04]);
    let not_waiting = branch(&mut code, 0x75);
    code.extend_from_slice(&[0xBF]);
    code.extend_from_slice(&2u32.to_le_bytes());
    let waiting = branch(&mut code, 0xEB);

    let choose_benchmark = code.len();
    patch(&mut code, not_waiting, choose_benchmark)?;
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x05]);
    let unknown_behaviour = branch(&mut code, 0x75);
    code.extend_from_slice(&[0x31, 0xFF]);
    code.extend_from_slice(&[0x41, 0xB8, 1, 0, 0, 0]);
    let benchmarking = branch(&mut code, 0xEB);

    let early_die = code.len();
    patch(&mut code, wrong_version, early_die)?;
    patch(&mut code, unknown_behaviour, early_die)?;
    code.extend_from_slice(&[0x0F, 0x0B, 0xEB, 0xFE]);

    // AC forces trap entry to establish SMAP isolation itself (D-101).
    let ring = code.len();
    patch(&mut code, echoing, ring)?;
    patch(&mut code, duplicating, ring)?;
    patch(&mut code, waiting, ring)?;
    patch(&mut code, benchmarking, ring)?;
    code.extend_from_slice(&[0x9C]);
    code.extend_from_slice(&[0x48, 0x81, 0x0C, 0x24, 0x00, 0x00, 0x04, 0x00]);
    code.extend_from_slice(&[0x9D]);

    code.extend_from_slice(&[0x48, 0xBB]);
    code.extend_from_slice(&RING.to_le_bytes());

    code.extend_from_slice(&[0xC7, 0x83]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 4).to_le_bytes());
    code.extend_from_slice(&PROGRAM_TAG.to_le_bytes());

    code.extend_from_slice(&[0x85, 0xFF]);
    let non_echo_submission = branch(&mut code, 0x75);

    code.extend_from_slice(&[0xC7, 0x83]);
    code.extend_from_slice(&SUBMISSIONS_AT.to_le_bytes());
    code.extend_from_slice(&ECHO.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xC7, 0x83]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 24).to_le_bytes());
    code.extend_from_slice(&PROGRAM_ARGUMENT.to_le_bytes());
    let echo_ready = branch(&mut code, 0xEB);

    let choose_submission = code.len();
    patch(&mut code, non_echo_submission, choose_submission)?;
    code.extend_from_slice(&[0x83, 0xFF, 0x01]);
    let timer_submission = branch(&mut code, 0x75);

    // The entry block, not the image, supplies bootstrap authority.
    code.extend_from_slice(&[0xC7, 0x83]);
    code.extend_from_slice(&SUBMISSIONS_AT.to_le_bytes());
    code.extend_from_slice(&DUPLICATE.to_le_bytes());
    code.extend_from_slice(&[0x89, 0xB3]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 8).to_le_bytes());
    code.extend_from_slice(&[0x48, 0xC7, 0x83]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 24).to_le_bytes());
    code.extend_from_slice(&DUPLICATE_RIGHTS.to_le_bytes());
    let duplicate_ready = branch(&mut code, 0xEB);

    // A short timer forces the distinct RingWait wake path.
    let timer = code.len();
    patch(&mut code, timer_submission, timer)?;
    code.extend_from_slice(&[0xC7, 0x83]);
    code.extend_from_slice(&SUBMISSIONS_AT.to_le_bytes());
    code.extend_from_slice(&TIMER.to_le_bytes());
    code.extend_from_slice(&[0x48, 0xC7, 0x83]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 24).to_le_bytes());
    code.extend_from_slice(&2_000_000u32.to_le_bytes());

    let publish = code.len();
    patch(&mut code, echo_ready, publish)?;
    patch(&mut code, duplicate_ready, publish)?;
    code.extend_from_slice(&[0xC7, 0x43, SUBMISSION_TAIL_AT]);
    code.extend_from_slice(&1u32.to_le_bytes());

    // A 32-bit delta tolerates one low-half TSC wrap during the short trip.
    code.extend_from_slice(&[0x45, 0x85, 0xC0]);
    let ordinary_ring_enter = branch(&mut code, 0x74);
    code.extend_from_slice(&[0x0F, 0xAE, 0xE8, 0x0F, 0x31]);
    code.extend_from_slice(&[0x41, 0x89, 0xC2]);

    let ring_enter = code.len();
    patch(&mut code, ordinary_ring_enter, ring_enter)?;
    code.extend_from_slice(&[0xB8]);
    code.extend_from_slice(&RING_ENTER.to_le_bytes());
    code.extend_from_slice(&[0xCD, 0x80]);

    // Timer admission completes nothing, so only RingWait may report it.
    code.extend_from_slice(&[0x83, 0xFF, 0x02]);
    let immediate = branch(&mut code, 0x75);
    code.extend_from_slice(&[0x48, 0x85, 0xC0]);
    let completed_early = branch(&mut code, 0x75);
    code.extend_from_slice(&[0xB8]);
    code.extend_from_slice(&RING_WAIT.to_le_bytes());
    code.extend_from_slice(&[0xCD, 0x80]);

    let expect_one = code.len();
    patch(&mut code, immediate, expect_one)?;
    code.extend_from_slice(&[0x48, 0x83, 0xF8, 0x01]);
    let served = branch(&mut code, 0x75);

    code.extend_from_slice(&[0x45, 0x85, 0xC0]);
    let not_benchmarking = branch(&mut code, 0x74);
    code.extend_from_slice(&[0x0F, 0xAE, 0xE8, 0x0F, 0x31]);
    code.extend_from_slice(&[0x44, 0x29, 0xD0]);
    code.extend_from_slice(&[0x89, 0x83]);
    code.extend_from_slice(&(SUBMISSIONS_AT + 16).to_le_bytes());

    // Aligned x86 `mov` supplies the ABI's acquire edge here (D-221).
    let acquire_completion = code.len();
    patch(&mut code, not_benchmarking, acquire_completion)?;
    code.extend_from_slice(&[0x8B, 0x43, COMPLETION_TAIL_AT]);
    code.extend_from_slice(&[0x83, 0xF8, 0x01]);
    let completion_published = branch(&mut code, 0x75);

    code.extend_from_slice(&[0x8B, 0x8B]);
    code.extend_from_slice(&(COMPLETIONS_AT + 4).to_le_bytes());
    code.extend_from_slice(&[0x85, 0xC9]);
    let clean = branch(&mut code, 0x75);

    code.extend_from_slice(&[0x48, 0x8B, 0x8B]);
    code.extend_from_slice(&(COMPLETIONS_AT + 8).to_le_bytes());
    code.extend_from_slice(&[0x85, 0xFF]);
    let non_echo_answer = branch(&mut code, 0x75);
    code.extend_from_slice(&[0x48, 0xC7, 0xC2]);
    code.extend_from_slice(&PROGRAM_ARGUMENT.to_le_bytes());
    code.extend_from_slice(&[0x48, 0x39, 0xD1]);
    let echoed = branch(&mut code, 0x75);
    let answered = branch(&mut code, 0xEB);

    let choose_answer = code.len();
    patch(&mut code, non_echo_answer, choose_answer)?;
    code.extend_from_slice(&[0x83, 0xFF, 0x02]);
    let timer_answer = branch(&mut code, 0x74);
    code.extend_from_slice(&[0x48, 0x85, 0xC9]);
    let empty_handle = branch(&mut code, 0x74);
    code.extend_from_slice(&[0x39, 0xF1]);
    let repeated_handle = branch(&mut code, 0x74);
    let duplicated = branch(&mut code, 0xEB);

    let timer_value = code.len();
    patch(&mut code, timer_answer, timer_value)?;
    code.extend_from_slice(&[0x48, 0x85, 0xC9]);
    let empty_deadline = branch(&mut code, 0x74);

    let exit = code.len();
    patch(&mut code, answered, exit)?;
    patch(&mut code, duplicated, exit)?;
    // The aligned store releases the consumed slot after its payload read.
    code.extend_from_slice(&[0xC7, 0x43, COMPLETION_HEAD_AT]);
    code.extend_from_slice(&1u32.to_le_bytes());
    code.extend_from_slice(&[0x31, 0xC0]);
    code.extend_from_slice(&[0xCD, 0x80]);

    let die = code.len();
    code.extend_from_slice(&[0x0F, 0x0B]);
    code.extend_from_slice(&[0xEB, 0xFE]);

    for site in [
        completed_early,
        served,
        completion_published,
        clean,
        echoed,
        empty_handle,
        repeated_handle,
        empty_deadline,
    ] {
        patch(&mut code, site, die)?;
    }
    Ok(code)
}

const PROGRAM_TAG: u32 = 0x0000_1234;

const PROGRAM_ARGUMENT: u32 = 0x00C0_FFEE;

/// Rights the ring-3 Duplicate asks to retain: universal INSPECT only.
const DUPLICATE_RIGHTS: u32 = 1 << 3;

/// A call number this kernel does not serve.
///
/// Far above anything `enum wiz_call` names, so adding a call cannot turn this
/// into a real one by accident.
const UNSERVED_CALL: u32 = 0x0BAD_0BAD;

fn branch(code: &mut Vec<u8>, opcode: u8) -> usize {
    code.push(opcode);
    code.push(0);
    code.len() - 1
}

fn patch(code: &mut [u8], site: usize, target: usize) -> Result<()> {
    let code_len = code.len();
    let from = site
        .checked_add(1)
        .context("computing the fixed witness branch origin")?;
    let slot = code.get_mut(site).with_context(|| {
        format!("fixed witness branch site {site} is outside its {code_len}-byte program")
    })?;
    let displacement = if target >= from {
        let distance = target - from;
        i8::try_from(distance).with_context(|| {
            format!("fixed witness jump from {from} to {target} exceeds rel8 range")
        })?
    } else {
        let distance = from - target;
        let distance = i16::try_from(distance).with_context(|| {
            format!("fixed witness jump from {from} to {target} exceeds rel8 range")
        })?;
        i8::try_from(-distance).with_context(|| {
            format!("fixed witness jump from {from} to {target} exceeds rel8 range")
        })?
    };
    *slot = displacement as u8;
    Ok(())
}

const PT_LOAD: u32 = 1;

/// Readable and executable, and deliberately not writable: W^X is not relaxed
/// for user memory (D-12).
const PF_READ_EXECUTE: u32 = 0b101;

/// Build the program image.
///
/// The segment uses byte alignment because the loader copies into fresh frames.
///
/// # Errors
///
/// Returns an error if the fixed program's branch layout exceeds `rel8`.
pub fn build() -> Result<Vec<u8>> {
    let code = code()?;
    let mut image = vec![0u8; HEADER_BYTES + PROGRAM_HEADER_BYTES + code.len()];

    image[0..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
    image[4] = 2;
    image[5] = 1;
    image[6] = 1;
    image[16..18].copy_from_slice(&2u16.to_le_bytes());
    image[18..20].copy_from_slice(&0x3Eu16.to_le_bytes());
    image[20..24].copy_from_slice(&1u32.to_le_bytes());
    image[24..32].copy_from_slice(&LOAD_ADDRESS.to_le_bytes());
    image[32..40].copy_from_slice(&(HEADER_BYTES as u64).to_le_bytes());
    image[52..54].copy_from_slice(&(HEADER_BYTES as u16).to_le_bytes());
    image[54..56].copy_from_slice(&(PROGRAM_HEADER_BYTES as u16).to_le_bytes());
    image[56..58].copy_from_slice(&1u16.to_le_bytes());

    let code_at = HEADER_BYTES + PROGRAM_HEADER_BYTES;
    let at = HEADER_BYTES;
    image[at..at + 4].copy_from_slice(&PT_LOAD.to_le_bytes());
    image[at + 4..at + 8].copy_from_slice(&PF_READ_EXECUTE.to_le_bytes());
    image[at + 8..at + 16].copy_from_slice(&(code_at as u64).to_le_bytes());
    image[at + 16..at + 24].copy_from_slice(&LOAD_ADDRESS.to_le_bytes());
    image[at + 24..at + 32].copy_from_slice(&LOAD_ADDRESS.to_le_bytes());
    image[at + 32..at + 40].copy_from_slice(&(code.len() as u64).to_le_bytes());
    image[at + 40..at + 48].copy_from_slice(&(code.len() as u64).to_le_bytes());
    image[at + 48..at + 56].copy_from_slice(&1u64.to_le_bytes());

    image[code_at..code_at + code.len()].copy_from_slice(&code);
    Ok(image)
}

#[cfg(test)]
mod tests {
    use super::*;
    use elf64::{Elf, SegmentKind};

    #[test]
    fn the_program_parses_as_the_kernel_will_read_it() {
        let image = build().expect("building the test program");
        let code = code().expect("building the test program code");
        let elf = Elf::parse(&image).expect("the built image is an elf64");

        assert_eq!(elf.entry(), LOAD_ADDRESS);
        assert_eq!(elf.segment_count(), 1);

        let segment = core::iter::Iterator::next(&mut elf.segments()).expect("one segment");
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
        let segment =
            core::iter::Iterator::next(&mut Elf::parse(&image).expect("an elf64").segments())
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
        let segment =
            core::iter::Iterator::next(&mut Elf::parse(&image).expect("an elf64").segments())
                .expect("one segment");

        let at = segment.offset() as usize;
        let len = segment.file_size() as usize;
        assert_eq!(&image[at..at + len], code.as_slice());
    }

    #[test]
    fn the_emitted_program_has_required_routes() {
        let code = code().expect("building the test program code");

        assert_eq!(
            &code[..12],
            &[
                0x59, 0x48, 0x83, 0xF9, 0x01, 0x75, code[6], 0x58, 0x5E, 0x48, 0x85, 0xC0,
            ],
            "pop rcx; cmp rcx, 1; jne die; pop rax; pop rsi; test rax, rax"
        );
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
        assert!(
            code.windows(4)
                .any(|bytes| bytes == [0x48, 0x83, 0xF8, 0x05]),
            "cmp rax, 5"
        );
        assert!(code.windows(2).any(|pair| pair == [0xFF, 0xE0]), "jmp rax");

        // Setting `AC`, which the entry path must clear. Searched for, not
        // indexed, so a behaviour added above moves the assertion with it.
        let flags = code
            .windows(10)
            .position(|w| w == [0x9C, 0x48, 0x81, 0x0C, 0x24, 0x00, 0x00, 0x04, 0x00, 0x9D])
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
}
