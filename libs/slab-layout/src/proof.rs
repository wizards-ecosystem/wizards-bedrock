use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

verus! {

impl crate::SizeClass {
    #[verifier::type_invariant]
    pub(crate) open spec fn in_table(self) -> bool {
        self.index < crate::CLASSES
    }

    pub open(crate) spec fn spec_index(self) -> usize {
        self.index
    }

    pub open(crate) spec fn spec_bytes(self) -> usize {
        class_bytes(self.index)
    }
}

pub open(crate) spec fn class_bytes(index: usize) -> usize {
    crate::MIN_CLASS << index
}

pub open spec fn is_power_of_two(value: usize) -> bool {
    exists|shift: u32| shift < usize::BITS && value == 1usize << shift
}

pub open(crate) spec fn fits(index: usize, size: usize, align: usize) -> bool {
    size <= class_bytes(index) && class_bytes(index) % align == 0
}

pub open(crate) spec fn smallest_fit(index: usize, size: usize, align: usize) -> bool {
    index < crate::CLASSES
        && fits(index, size, align)
        && forall|smaller: usize| smaller < index ==> !fits(smaller, size, align)
}

pub open(crate) spec fn no_fit(size: usize, align: usize) -> bool {
    forall|index: usize| index < crate::CLASSES ==> !fits(index, size, align)
}

pub open spec fn frames_covering(bytes: usize) -> int {
    (bytes + 4095) / 4096
}

pub(crate) proof fn lemma_class_bytes(index: usize)
    requires
        index < crate::CLASSES,
    ensures
        16 <= class_bytes(index) <= 2048,
{
    assert(index < 8 ==> 16 <= (16usize << index) && (16usize << index) <= 2048) by (bit_vector);
}

pub(crate) proof fn lemma_fit_by_order(index: usize, align: usize)
    requires
        index < crate::CLASSES,
        is_power_of_two(align),
    ensures
        (align <= class_bytes(index)) == (class_bytes(index) % align == 0),
{
    let shift = choose|shift: u32| shift < usize::BITS && align == 1usize << shift;
    assert(((1usize << shift) <= (16usize << index)) == ((16usize << index) % (1usize << shift)
        == 0)) by (bit_vector)
        requires
            index < 8,
            shift < usize::BITS,
    ;
}

pub(crate) proof fn lemma_slot_in_frame(slot: usize, bytes: usize)
    requires
        16 <= bytes <= 2048,
    ensures
        (slot < 4096usize / bytes) == (slot * bytes + bytes <= 4096),
{
    let (s, b) = (slot as int, bytes as int);
    let whole = 4096int / b;
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(4096, b);
    vstd::arithmetic::div_mod::lemma_mod_bound(4096, b);
    assert(b * whole == whole * b && s * b + b == (s + 1) * b && whole * b + b == (whole + 1) * b)
        by (nonlinear_arith);
    if s < whole {
        assert((s + 1) * b <= whole * b) by (nonlinear_arith)
            requires
                s + 1 <= whole,
                b > 0,
        ;
    } else {
        assert((whole + 1) * b <= (s + 1) * b) by (nonlinear_arith)
            requires
                whole <= s,
                b > 0,
        ;
    }
}

} // verus!
