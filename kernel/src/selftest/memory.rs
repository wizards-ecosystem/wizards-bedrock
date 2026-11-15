use wost::mm;

const DIRT: u8 = 0xAA;

pub(super) fn allocation_is_zeroed() -> bool {
    let Some(mut dirty) = mm::allocate() else {
        return false;
    };
    if !dirty.fill(DIRT) || dirty.is_zero() != Some(false) {
        return false;
    }
    drop(dirty);

    let Some(clean) = mm::allocate_zeroed() else {
        return false;
    };
    let Some(other) = mm::allocate() else {
        return false;
    };
    clean.is_zero() == Some(true) && other.address() != clean.address()
}
