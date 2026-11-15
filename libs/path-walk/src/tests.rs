use super::*;

#[test]
fn dot_segments_resolve_and_nothing_escapes() {
    let mut components = [b"".as_slice(); 8];

    let count = split(b"a/./b/../c", &mut components).expect("a valid path");
    assert_eq!(&components[..count], &[b"a".as_slice(), b"c".as_slice()]);

    assert_eq!(split(b"a/../..", &mut components), Err(PathError::Escapes));
    assert_eq!(split(b"/etc", &mut components), Err(PathError::Absolute));
}
