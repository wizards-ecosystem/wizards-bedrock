use super::*;

fn components(path: &[u8]) -> Result<alloc::vec::Vec<alloc::vec::Vec<u8>>, PathError> {
    let mut slots = [b"".as_slice(); 16];
    let count = split(path, &mut slots)?;
    Ok(slots[..count].iter().map(|part| part.to_vec()).collect())
}

extern crate alloc;

/// A walker fed these can only descend, because nothing in the output means up.
#[test]
fn dot_dot_cancels_and_never_survives() {
    assert_eq!(components(b"a/../b").unwrap(), vec![b"b".to_vec()]);
    assert_eq!(
        components(b"a/b/../c").unwrap(),
        vec![b"a".to_vec(), b"c".to_vec()]
    );
    assert_eq!(components(b"a/b/../..").unwrap(), Vec::<Vec<u8>>::new());
    assert_eq!(
        components(b"a/b/c/../../d").unwrap(),
        vec![b"a".to_vec(), b"d".to_vec()]
    );

    for path in [
        b"a/../b".as_slice(),
        b"a/b/../c",
        b"a/b/c/../../d",
        b"x/y/z/../../../q",
    ] {
        for part in components(path).unwrap() {
            assert_ne!(part, b"..".to_vec(), "{path:?} kept a ..");
            assert_ne!(part, b".".to_vec());
        }
    }
}
