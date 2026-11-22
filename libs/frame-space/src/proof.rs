use vstd::prelude::verus;
#[cfg(verus_only)]
use vstd::prelude::*;

verus! {

pub open spec fn bit_set(word: u64, bit: u64) -> bool {
    (word >> bit) & 1 == 1
}

// How many of the first `words` words have `bit` set: the frames at that bit position.
pub open spec fn column(map: Seq<u64>, bit: u64, words: nat) -> nat
    decreases words,
{
    if words == 0 {
        0
    } else {
        column(map, bit, (words - 1) as nat) + if bit_set(map[words - 1], bit) {
            1nat
        } else {
            0nat
        }
    }
}

// Every set bit at positions below `bits` in the first `words` words.
pub open spec fn population(map: Seq<u64>, bits: nat, words: nat) -> nat
    decreases bits,
{
    if bits == 0 {
        0
    } else {
        population(map, (bits - 1) as nat, words) + column(map, (bits - 1) as u64, words)
    }
}

// The counters at positions below `bits` whose position has residue `colour` modulo `stride`.
pub open spec fn strided(counts: Seq<usize>, stride: nat, colour: nat, bits: nat) -> nat
    decreases bits,
{
    if bits == 0 {
        0
    } else {
        strided(counts, stride, colour, (bits - 1) as nat) + if (bits - 1) as nat % stride
            == colour {
            counts[bits - 1] as nat
        } else {
            0nat
        }
    }
}

pub open spec fn domain_words(words: usize) -> usize {
    if words < usize::MAX / 64 {
        words
    } else {
        (usize::MAX / 64) as usize
    }
}

pub open spec fn frame_count(words: usize) -> int {
    domain_words(words) * 64
}

pub open spec fn is_power_of_two(value: usize) -> bool {
    exists|shift: u32| shift < usize::BITS && value == 1usize << shift
}

pub proof fn lemma_set_bit(word: u64, set: u64, bit: u64)
    requires
        set < 64,
        bit < 64,
    ensures
        bit_set(word | (1u64 << set), bit) == (bit_set(word, bit) || bit == set),
{
    assert(bit_set(word | (1u64 << set), bit) == (bit_set(word, bit) || bit == set)) by (bit_vector)
        requires
            set < 64,
            bit < 64,
    ;
}

pub proof fn lemma_clear_bit(word: u64, clear: u64, bit: u64)
    requires
        clear < 64,
        bit < 64,
    ensures
        bit_set(word & !(1u64 << clear), bit) == (bit_set(word, bit) && bit != clear),
{
    assert(bit_set(word & !(1u64 << clear), bit) == (bit_set(word, bit) && bit != clear)) by (bit_vector)
        requires
            clear < 64,
            bit < 64,
    ;
}

pub proof fn lemma_test_bit(word: u64, bit: u64)
    requires
        bit < 64,
    ensures
        (word & (1u64 << bit) != 0) == bit_set(word, bit),
{
    assert((word & (1u64 << bit) != 0) == bit_set(word, bit)) by (bit_vector)
        requires
            bit < 64,
    ;
}

pub proof fn lemma_subset_bit(small: u64, large: u64, bit: u64)
    requires
        small & !large == 0,
        bit < 64,
    ensures
        bit_set(small, bit) ==> bit_set(large, bit),
{
    assert(bit_set(small, bit) ==> bit_set(large, bit)) by (bit_vector)
        requires
            small & !large == 0,
            bit < 64,
    ;
}

pub proof fn lemma_subset_grows(small: u64, large: u64, set: u64)
    requires
        small & !large == 0,
        set < 64,
    ensures
        small & !(large | (1u64 << set)) == 0,
        bit_set(large, set) ==> (small | (1u64 << set)) & !large == 0,
        (small & !(1u64 << set)) & !large == 0,
{
    assert(small & !(large | (1u64 << set)) == 0) by (bit_vector)
        requires
            small & !large == 0,
            set < 64,
    ;
    assert(bit_set(large, set) ==> (small | (1u64 << set)) & !large == 0) by (bit_vector)
        requires
            small & !large == 0,
            set < 64,
    ;
    assert((small & !(1u64 << set)) & !large == 0) by (bit_vector)
        requires
            small & !large == 0,
    ;
}

pub proof fn lemma_column_below(map: Seq<u64>, index: int, word: u64, bit: u64, words: nat)
    requires
        words <= index < map.len(),
    ensures
        column(map.update(index, word), bit, words) == column(map, bit, words),
    decreases words,
{
    if words > 0 {
        lemma_column_below(map, index, word, bit, (words - 1) as nat);
    }
}

pub proof fn lemma_column_update(map: Seq<u64>, index: int, word: u64, bit: u64, words: nat)
    requires
        0 <= index < words <= map.len(),
    ensures
        column(map.update(index, word), bit, words) + (if bit_set(map[index], bit) {
            1int
        } else {
            0int
        }) == column(map, bit, words) + (if bit_set(word, bit) {
            1int
        } else {
            0int
        }),
    decreases words,
{
    if index == words - 1 {
        lemma_column_below(map, index, word, bit, (words - 1) as nat);
    } else {
        lemma_column_update(map, index, word, bit, (words - 1) as nat);
    }
}

pub proof fn lemma_column_bound(map: Seq<u64>, bit: u64, words: nat)
    ensures
        column(map, bit, words) <= words,
    decreases words,
{
    if words > 0 {
        lemma_column_bound(map, bit, (words - 1) as nat);
    }
}

pub proof fn lemma_column_subset(small: Seq<u64>, large: Seq<u64>, bit: u64, words: nat)
    requires
        bit < 64,
        words <= small.len(),
        words <= large.len(),
        forall|index: int| 0 <= index < words ==> #[trigger] (small[index] & !large[index]) == 0,
    ensures
        column(small, bit, words) <= column(large, bit, words),
    decreases words,
{
    if words > 0 {
        lemma_column_subset(small, large, bit, (words - 1) as nat);
        lemma_subset_bit(small[words - 1], large[words - 1], bit);
    }
}

pub proof fn lemma_column_member(map: Seq<u64>, bit: u64, words: nat)
    requires
        column(map, bit, words) > 0,
    ensures
        exists|index: int| 0 <= index < words && bit_set(#[trigger] map[index], bit),
    decreases words,
{
    if !bit_set(map[words - 1], bit) {
        lemma_column_member(map, bit, (words - 1) as nat);
        let index = choose|index: int| 0 <= index < words - 1 && bit_set(#[trigger] map[index], bit);
        assert(0 <= index < words && bit_set(map[index], bit));
    }
}

pub proof fn lemma_column_empty_tail(map: Seq<u64>, bit: u64, words: nat, used: nat)
    requires
        used <= words,
        forall|index: int| used <= index < words ==> !#[trigger] bit_set(map[index], bit),
    ensures
        column(map, bit, words) == column(map, bit, used),
    decreases words,
{
    if words > used {
        lemma_column_empty_tail(map, bit, (words - 1) as nat, used);
    }
}

pub proof fn lemma_population_change(before: Seq<u64>, after: Seq<u64>, words: nat, at: u64, bits: nat)
    requires
        at < 64,
        bits <= 64,
        forall|bit: u64| bit < 64 && bit != at ==> #[trigger] column(after, bit, words) == column(
            before,
            bit,
            words,
        ),
    ensures
        population(after, bits, words) + (if at < bits {
            column(before, at, words) as int
        } else {
            0int
        }) == population(before, bits, words) + (if at < bits {
            column(after, at, words) as int
        } else {
            0int
        }),
    decreases bits,
{
    if bits > 0 {
        lemma_population_change(before, after, words, at, (bits - 1) as nat);
        assert(column(after, (bits - 1) as u64, words) == column(before, (bits - 1) as u64, words)
            || (bits - 1) as u64 == at);
    }
}

pub proof fn lemma_population_subset(small: Seq<u64>, large: Seq<u64>, words: nat, bits: nat)
    requires
        bits <= 64,
        words <= small.len(),
        words <= large.len(),
        forall|index: int| 0 <= index < words ==> #[trigger] (small[index] & !large[index]) == 0,
    ensures
        population(small, bits, words) <= population(large, bits, words),
    decreases bits,
{
    if bits > 0 {
        lemma_population_subset(small, large, words, (bits - 1) as nat);
        lemma_column_subset(small, large, (bits - 1) as u64, words);
    }
}

pub proof fn lemma_population_bound(map: Seq<u64>, bits: nat, words: nat, used: nat)
    requires
        bits <= 64,
        used <= words,
        forall|bit: u64, index: int|
            bit < 64 && used <= index < words ==> !#[trigger] bit_set(map[index], bit),
    ensures
        population(map, bits, words) <= bits * used,
    decreases bits,
{
    if bits > 0 {
        lemma_population_bound(map, (bits - 1) as nat, words, used);
        let bit = (bits - 1) as u64;
        assert forall|index: int| used <= index < words implies !#[trigger] bit_set(map[index], bit) by {}
        lemma_column_empty_tail(map, bit, words, used);
        lemma_column_bound(map, bit, used);
        assert(bits * used == (bits - 1) * used + used) by (nonlinear_arith);
    }
}

pub proof fn lemma_strided_change(
    before: Seq<usize>,
    after: Seq<usize>,
    stride: nat,
    colour: nat,
    at: int,
    bits: nat,
)
    requires
        stride > 0,
        bits <= before.len(),
        bits <= after.len(),
        0 <= at,
        forall|bit: int| 0 <= bit < bits && bit != at ==> #[trigger] after[bit] == before[bit],
    ensures
        strided(after, stride, colour, bits) + (if at < bits && at % stride as int == colour {
            before[at] as int
        } else {
            0int
        }) == strided(before, stride, colour, bits) + (if at < bits && at % stride as int
            == colour {
            after[at] as int
        } else {
            0int
        }),
    decreases bits,
{
    if bits > 0 {
        lemma_strided_change(before, after, stride, colour, at, (bits - 1) as nat);
    }
}

pub proof fn lemma_strided_member(counts: Seq<usize>, stride: nat, colour: nat, bits: nat)
    requires
        stride > 0,
        bits <= counts.len(),
        strided(counts, stride, colour, bits) > 0,
    ensures
        exists|bit: int|
            0 <= bit < bits && bit % stride as int == colour && #[trigger] counts[bit] > 0,
    decreases bits,
{
    let last = bits - 1;
    if !(last % stride as int == colour && counts[last] > 0) {
        lemma_strided_member(counts, stride, colour, (bits - 1) as nat);
        let bit = choose|bit: int|
            0 <= bit < bits - 1 && bit % stride as int == colour && #[trigger] counts[bit] > 0;
        assert(0 <= bit < bits && bit % stride as int == colour && counts[bit] > 0);
    }
}

pub proof fn lemma_strided_bound(counts: Seq<usize>, stride: nat, colour: nat, bits: nat)
    requires
        stride > 0,
        bits <= counts.len(),
    ensures
        strided(counts, stride, colour, bits) <= strided(counts, 1, 0, bits),
    decreases bits,
{
    if bits > 0 {
        lemma_strided_bound(counts, stride, colour, (bits - 1) as nat);
        assert((bits - 1) as nat % 1 == 0);
    }
}

pub proof fn lemma_strided_columns(counts: Seq<usize>, map: Seq<u64>, words: nat, bits: nat)
    requires
        bits <= 64,
        bits <= counts.len(),
        forall|bit: int| 0 <= bit < bits ==> #[trigger] counts[bit] == column(map, bit as u64, words),
    ensures
        strided(counts, 1, 0, bits) == population(map, bits, words),
    decreases bits,
{
    if bits > 0 {
        lemma_strided_columns(counts, map, words, (bits - 1) as nat);
        assert((bits - 1) as nat % 1 == 0);
    }
}

pub proof fn lemma_strided_prefix(counts: Seq<usize>, stride: nat, colour: nat, bits: nat, more: nat)
    requires
        stride > 0,
        bits <= more,
        more <= counts.len(),
    ensures
        strided(counts, stride, colour, bits) <= strided(counts, stride, colour, more),
    decreases more,
{
    if more > bits {
        lemma_strided_prefix(counts, stride, colour, bits, (more - 1) as nat);
    }
}

// From one position of residue `colour` to the next, the strided sum gains only that position.
pub proof fn lemma_strided_step(counts: Seq<usize>, stride: nat, colour: nat, bit: nat, steps: nat)
    requires
        stride > 0,
        colour < stride,
        bit % stride == colour,
        steps <= stride,
        bit + steps <= counts.len(),
    ensures
        strided(counts, stride, colour, bit + steps) == strided(counts, stride, colour, bit)
            + if steps > 0 {
            counts[bit as int] as nat
        } else {
            0nat
        },
    decreases steps,
{
    if steps > 1 {
        lemma_strided_step(counts, stride, colour, bit, (steps - 1) as nat);
        lemma_residue_moves(bit as int, (steps - 1) as int, stride as int);
    }
}

pub proof fn lemma_residue_moves(bit: int, step: int, stride: int)
    requires
        0 <= bit,
        0 < step < stride,
    ensures
        (bit + step) % stride != bit % stride,
{
    let (low, high) = (bit / stride, (bit + step) / stride);
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(bit, stride);
    vstd::arithmetic::div_mod::lemma_fundamental_div_mod(bit + step, stride);
    if (bit + step) % stride == bit % stride {
        assert(stride * (high - low) == step) by (nonlinear_arith)
            requires
                bit == stride * low + bit % stride,
                bit + step == stride * high + (bit + step) % stride,
                (bit + step) % stride == bit % stride,
        ;
        assert(false) by (nonlinear_arith)
            requires
                stride * (high - low) == step,
                0 < step < stride,
        ;
    }
}

pub proof fn lemma_low_mask(frame: usize, count: usize)
    requires
        is_power_of_two(count),
        count <= 64,
    ensures
        frame & (count - 1) as usize == frame % count,
{
    let shift = choose|shift: u32| shift < usize::BITS && count == 1usize << shift;
    assert(frame & ((1usize << shift) - 1) as usize == frame % (1usize << shift)) by (bit_vector)
        requires
            shift < usize::BITS,
    ;
}

pub proof fn lemma_full_mask(count: u64)
    requires
        0 < count <= 64,
    ensures
        forall|bit: u64| bit < 64 ==> #[trigger] bit_set(u64::MAX >> ((64 - count) as u64), bit) == (bit < count),
{
    assert forall|bit: u64| bit < 64 implies #[trigger] bit_set(u64::MAX >> ((64 - count) as u64), bit) == (bit
        < count) by {
        assert(bit_set(u64::MAX >> ((64 - count) as u64), bit) == (bit < count)) by (bit_vector)
            requires
                0 < count <= 64,
                bit < 64,
        ;
    }
}

pub proof fn lemma_full_pattern()
    ensures
        forall|bit: u64| bit < 64 ==> #[trigger] bit_set(u64::MAX, bit),
{
    assert forall|bit: u64| bit < 64 implies #[trigger] bit_set(u64::MAX, bit) by {
        assert(bit_set(u64::MAX, bit)) by (bit_vector)
            requires
                bit < 64,
        ;
    }
}

pub proof fn lemma_domain_index(words: usize, to: int, frame: int)
    requires
        0 <= frame < to <= frame_count(words),
    ensures
        frame / 64 < domain_words(words) <= words,
        frame + 64 <= usize::MAX,
{
}

pub proof fn lemma_lowest_bit(word: u64)
    requires
        word != 0,
    ensures
        word.trailing_zeros() < 64,
        bit_set(word, word.trailing_zeros() as u64),
        forall|bit: u64| bit < word.trailing_zeros() ==> !#[trigger] bit_set(word, bit),
{
    vstd::std_specs::bits::axiom_u64_trailing_zeros(word);
}

pub proof fn lemma_align_mask(value: usize, align: usize, sum: usize)
    requires
        is_power_of_two(align),
        sum == value + (align - 1),
    ensures
        value <= (sum & !((align - 1) as usize)),
        (sum & !((align - 1) as usize)) % align == 0,
{
    lemma_power_positive(align);
    let shift = choose|shift: u32| shift < usize::BITS && align == 1usize << shift;
    let mask = ((1usize << shift) - 1) as usize;
    let rounded = sum & !mask;
    assert(value <= rounded && rounded & mask == 0) by (bit_vector)
        requires
            shift < usize::BITS,
            mask == ((1usize << shift) - 1) as usize,
            sum == (value + mask) as usize,
            sum >= value,
            rounded == sum & !mask,
    ;
    lemma_power_mask(align, shift);
    vstd::bits::lemma_usize_low_bits_mask_is_mod(rounded, shift as nat);
}

// A power of two is `pow2` of its shift, and one less is the low-bits mask.
pub proof fn lemma_power_mask(align: usize, shift: u32)
    requires
        shift < usize::BITS,
        align == 1usize << shift,
    ensures
        align as nat == vstd::arithmetic::power2::pow2(shift as nat),
        (align - 1) as usize as nat == vstd::bits::low_bits_mask(shift as nat),
{
    assert(1usize << shift == 1usize << (shift as usize)) by (bit_vector)
        requires
            shift < usize::BITS,
    ;
    vstd::bits::lemma_usize_pow2_no_overflow(shift as nat);
    vstd::bits::lemma_usize_shl_is_mul(1, shift as usize);
}

pub open spec fn can_seal(owner: crate::Owner) -> bool {
    match owner {
        crate::Owner::Kernel => true,
        crate::Owner::Arena(_, crate::FrameKind::ArenaObject) => true,
        crate::Owner::Arena(_, crate::FrameKind::ArenaData) => true,
        _ => false,
    }
}

// A surrendered frame's link fits its 26 bits.
pub open spec fn link_fits(owner: crate::Owner) -> bool {
    match owner {
        crate::Owner::Surrendered(_, _, next) => next.spec_get() <= 0x3ff_ffff,
        _ => true,
    }
}

pub open spec fn well_formed(record: crate::Record) -> bool {
    &&& (record.spec_access() == crate::Access::Sealed ==> can_seal(record.spec_owner()))
    &&& link_fits(record.spec_owner())
}

// Only a writable frame of `arena` is surrendered, so each frame joins one list once.
pub open spec fn surrenders(from: crate::Record, arena: crate::ArenaIndex, next: crate::Link) -> bool {
    &&& from.spec_access() == crate::Access::Writable
    &&& next.spec_get() <= 0x3ff_ffff
    &&& match from.spec_owner() {
        crate::Owner::Arena(index, _) => index == arena,
        _ => false,
    }
}

pub open spec fn surrender_of(owner: crate::Owner, next: crate::Link) -> crate::Owner {
    match owner {
        crate::Owner::Arena(arena, kind) => crate::Owner::Surrendered(arena, kind, next),
        owner => owner,
    }
}

pub open spec fn kind_bits(kind: crate::FrameKind) -> u64 {
    match kind {
        crate::FrameKind::ArenaObject => 0,
        crate::FrameKind::ArenaData => 1,
        crate::FrameKind::Table => 2,
    }
}

pub open spec fn class_bits(owner: crate::Owner) -> u64 {
    match owner {
        crate::Owner::Free => 0,
        crate::Owner::Kernel => 1,
        crate::Owner::Table => 2,
        crate::Owner::Device => 3,
        crate::Owner::Arena(_, _) => 4,
        crate::Owner::Surrendered(_, _, _) => 5,
    }
}

pub open spec fn owner_word(owner: crate::Owner) -> u64 {
    match owner {
        crate::Owner::Arena(arena, kind) => 4u64 | kind_bits(kind) << 4u64 | (
        arena.spec_get() as u64) << 32u64,
        crate::Owner::Surrendered(arena, kind, next) => 5u64 | kind_bits(kind) << 4u64 | (
        next.spec_get() as u64) << 6u64 | (arena.spec_get() as u64) << 32u64,
        owner => class_bits(owner),
    }
}

// The record word: owner class in bits 0 to 2, sealed in bit 3, for an arena or surrendered owner
// the frame kind in bits 4 and 5 and the arena index in bits 32 to 63, and for a surrendered one
// the link in bits 6 to 31.
pub open spec fn encoded(record: crate::Record) -> u64 {
    match record.spec_access() {
        crate::Access::Writable => owner_word(record.spec_owner()),
        crate::Access::Sealed => owner_word(record.spec_owner()) | 8u64,
    }
}

pub open spec fn layout_matches(word: u64, record: crate::Record) -> bool {
    &&& word & 7 == class_bits(record.spec_owner())
    &&& (word & 8 == 0) == (record.spec_access() == crate::Access::Writable)
    &&& match record.spec_owner() {
        crate::Owner::Arena(arena, kind) => (word & 0x30) >> 4u64 == kind_bits(kind) && (word
            >> 32u64) as u32 == arena.spec_get() && word & 0xffff_ffc0u64 == 0,
        crate::Owner::Surrendered(arena, kind, next) => (word & 0x30) >> 4u64 == kind_bits(kind)
            && (word >> 32u64) as u32 == arena.spec_get() && (word & 0xffff_ffc0u64) >> 6u64
            == next.spec_get() as u64,
        _ => word & (0x30u64 | 0xffff_ffff_0000_0000u64) == 0 && word & 0xffff_ffc0u64 == 0,
    }
}

pub proof fn lemma_encoded_layout(record: crate::Record)
    requires
        link_fits(record.spec_owner()),
    ensures
        layout_matches(encoded(record), record),
{
    match record.spec_owner() {
        crate::Owner::Arena(arena, kind) => {
            let (k, a) = (kind_bits(kind), arena.spec_get());
            let plain = 4u64 | k << 4u64 | (a as u64) << 32u64;
            assert(plain & 0xffff_ffc0u64 == 0 && plain & 7 == 4 && plain & 8 == 0 && (plain
                & 0x30) >> 4u64 == k && (plain >> 32u64) as u32 == a && (plain | 8u64)
                & 0xffff_ffc0u64 == 0 && (plain | 8u64) & 7 == 4 && (plain | 8u64) & 8 != 0 && ((
            plain | 8u64) & 0x30) >> 4u64 == k && ((plain | 8u64) >> 32u64) as u32 == a)
                by (bit_vector)
                requires
                    k <= 2,
                    plain == 4u64 | k << 4u64 | (a as u64) << 32u64,
            ;
        },
        crate::Owner::Surrendered(arena, kind, next) => {
            let (k, a, l) = (kind_bits(kind), arena.spec_get(), next.spec_get());
            let plain = 5u64 | k << 4u64 | (l as u64) << 6u64 | (a as u64) << 32u64;
            assert(plain & 7 == 5 && plain & 8 == 0 && (plain & 0x30) >> 4u64 == k && (plain
                >> 32u64) as u32 == a && (plain & 0xffff_ffc0u64) >> 6u64 == l as u64 && (plain
                | 8u64) & 7 == 5 && (plain | 8u64) & 8 != 0 && ((plain | 8u64) & 0x30) >> 4u64 == k
                && ((plain | 8u64) >> 32u64) as u32 == a && ((plain | 8u64) & 0xffff_ffc0u64)
                >> 6u64 == l as u64) by (bit_vector)
                requires
                    k <= 2,
                    l <= 0x3ff_ffff,
                    plain == 5u64 | k << 4u64 | (l as u64) << 6u64 | (a as u64) << 32u64,
            ;
        },
        owner => {
            let c = class_bits(owner);
            assert(c <= 3 ==> c & 0xffff_ffc0u64 == 0 && c & 7 == c && c & 8 == 0 && c & (0x30u64
                | 0xffff_ffff_0000_0000u64) == 0 && (c | 8u64) & 0xffff_ffc0u64 == 0 && (c | 8u64)
                & 7 == c && (c | 8u64) & 8 != 0 && (c | 8u64) & (0x30u64
                | 0xffff_ffff_0000_0000u64) == 0) by (bit_vector);
        },
    }
}

pub proof fn lemma_layout_unique(word: u64, record: crate::Record)
    requires
        layout_matches(word, record),
    ensures
        encoded(record) == word,
        forall|other: crate::Record|
            link_fits(other.spec_owner()) && #[trigger] encoded(other) == word ==> other == record,
{
    assert((word & 0xffff_ffc0u64) >> 6u64 <= 0x3ff_ffffu64) by (bit_vector);
    lemma_encoded_layout(record);
    let e = encoded(record);
    assert(word & 7 == e & 7 && (word & 8 == 0) == (e & 8 == 0) && (word & 0x30) >> 4u64 == (e
        & 0x30) >> 4u64 && (word & 0xffff_ffc0u64) >> 6u64 == (e & 0xffff_ffc0u64) >> 6u64 && (word
        >> 32u64) as u32 == (e >> 32u64) as u32 ==> word == e) by (bit_vector);
    assert(word & (0x30u64 | 0xffff_ffff_0000_0000u64) == 0 ==> (word & 0x30) >> 4u64 == 0 && (word
        >> 32u64) as u32 == 0) by (bit_vector);
    assert(e & (0x30u64 | 0xffff_ffff_0000_0000u64) == 0 ==> (e & 0x30) >> 4u64 == 0 && (e
        >> 32u64) as u32 == 0) by (bit_vector);
    assert(word & 0xffff_ffc0u64 == 0 ==> (word & 0xffff_ffc0u64) >> 6u64 == 0) by (bit_vector);
    assert(e & 0xffff_ffc0u64 == 0 ==> (e & 0xffff_ffc0u64) >> 6u64 == 0) by (bit_vector);
    assert forall|other: crate::Record|
        link_fits(other.spec_owner()) && #[trigger] encoded(other) == word implies other
        == record by {
        lemma_encoded_layout(other);
        record.lemma_same_layout(other, word);
    }
}

pub proof fn lemma_no_encoding(word: u64)
    requires
        (word & 7 != 5 && word & 0xffff_ffc0u64 != 0) || word & 7 > 5 || (word & 7 < 4 && word
            & (0x30u64 | 0xffff_ffff_0000_0000u64) != 0) || (word & 7 >= 4 && (word & 0x30)
            >> 4u64 == 3),
    ensures
        forall|record: crate::Record|
            link_fits(record.spec_owner()) ==> #[trigger] encoded(record) != word,
{
    assert forall|record: crate::Record|
        link_fits(record.spec_owner()) implies #[trigger] encoded(record) != word by {
        lemma_encoded_layout(record);
    }
}


pub open spec fn frame_in(map: Seq<u64>, frame: int) -> bool {
    0 <= frame && frame / 64 < map.len() && bit_set(map[frame / 64], (frame % 64) as u64)
}

pub open spec fn with_bit(map: Seq<u64>, frame: int) -> Seq<u64> {
    map.update(frame / 64, map[frame / 64] | (1u64 << (frame % 64) as u64))
}

pub open spec fn without_bit(map: Seq<u64>, frame: int) -> Seq<u64> {
    map.update(frame / 64, map[frame / 64] & !(1u64 << (frame % 64) as u64))
}

// The bitmap invariant: free frames are managed frames, the counters are the bitmaps' counts, and
// every managed frame lies below the limit, inside the frame domain.
pub open spec fn owned_exactly(
    words: usize,
    managed: Seq<u64>,
    free: Seq<u64>,
    by_bit: Seq<usize>,
    count: usize,
    limit: usize,
) -> bool {
    &&& managed.len() == words
    &&& free.len() == words
    &&& by_bit.len() == 64
    &&& forall|index: int| 0 <= index < words ==> #[trigger] (free[index] & !managed[index]) == 0
    &&& count == population(managed, 64, words as nat)
    &&& forall|bit: int|
        0 <= bit < 64 ==> #[trigger] by_bit[bit] == column(free, bit as u64, words as nat)
    &&& forall|frame: int| #[trigger] frame_in(managed, frame) ==> frame < limit
    &&& limit <= frame_count(words)
}

pub open spec fn inserted(words: usize, base: u64, length: u64, frame: int) -> bool {
    &&& base + length <= u64::MAX
    &&& (base + 4095) / 4096 <= frame
    &&& frame < (base + length) / 4096
    &&& frame < frame_count(words)
}

pub proof fn lemma_and_bit(left: u64, right: u64, bit: u64)
    requires
        bit < 64,
    ensures
        bit_set(left & right, bit) == (bit_set(left, bit) && bit_set(right, bit)),
{
    assert(bit_set(left & right, bit) == (bit_set(left, bit) && bit_set(right, bit))) by (bit_vector)
        requires
            bit < 64,
    ;
}

pub proof fn lemma_shift_or(pattern: u64, width: u64, bit: u64)
    requires
        width < 64,
        bit < 64,
    ensures
        bit_set(pattern | (pattern << width), bit) == (bit_set(pattern, bit) || (bit >= width && bit_set(
            pattern,
            (bit - width) as u64,
        ))),
{
    assert(bit_set(pattern | (pattern << width), bit) == (bit_set(pattern, bit) || (bit >= width && bit_set(
        pattern,
        (bit - width) as u64,
    )))) by (bit_vector)
        requires
            width < 64,
            bit < 64,
    ;
}

pub proof fn lemma_and_shift(word: u64, pattern: u64, shift: u64, bit: u64)
    requires
        shift < 64,
        bit < 64,
    ensures
        bit_set(word & pattern & (u64::MAX << shift), bit) == (bit_set(word, bit) && bit_set(pattern, bit)
            && bit >= shift),
{
    assert(bit_set(word & pattern & (u64::MAX << shift), bit) == (bit_set(word, bit) && bit_set(pattern, bit)
        && bit >= shift)) by (bit_vector)
        requires
            shift < 64,
            bit < 64,
    ;
}

pub proof fn lemma_not_shift(word: u64, shift: u64, bit: u64)
    requires
        shift < 64,
        bit < 64,
    ensures
        bit_set((!word) >> shift, bit) == (shift + bit < 64 && !bit_set(word, (shift + bit) as u64)),
{
    assert(bit_set((!word) >> shift, bit) == (shift + bit < 64 && !bit_set(word, (shift + bit) as u64)))
        by (bit_vector)
        requires
            shift < 64,
            bit < 64,
    ;
}

pub proof fn lemma_zero_word(bit: u64)
    ensures
        !bit_set(0, bit),
{
    assert(!bit_set(0, bit)) by (bit_vector);
}

pub proof fn lemma_frames_with_bit(map: Seq<u64>, frame: int)
    requires
        0 <= frame,
        frame / 64 < map.len(),
    ensures
        forall|other: int| #[trigger]
            frame_in(with_bit(map, frame), other) == (frame_in(map, other) || other == frame),
{
    assert forall|other: int| #[trigger]
        frame_in(with_bit(map, frame), other) == (frame_in(map, other) || other == frame) by {
        if 0 <= other && other / 64 == frame / 64 {
            lemma_set_bit(map[frame / 64], (frame % 64) as u64, (other % 64) as u64);
        }
    }
}

pub proof fn lemma_frames_without_bit(map: Seq<u64>, frame: int)
    requires
        0 <= frame,
        frame / 64 < map.len(),
    ensures
        forall|other: int| #[trigger]
            frame_in(without_bit(map, frame), other) == (frame_in(map, other) && other != frame),
{
    assert forall|other: int| #[trigger]
        frame_in(without_bit(map, frame), other) == (frame_in(map, other) && other != frame) by {
        if 0 <= other && other / 64 == frame / 64 {
            lemma_clear_bit(map[frame / 64], (frame % 64) as u64, (other % 64) as u64);
        }
    }
}

pub proof fn lemma_column_with_bit(map: Seq<u64>, frame: int, words: nat)
    requires
        0 <= frame,
        frame / 64 < words == map.len(),
        !frame_in(map, frame),
    ensures
        forall|bit: u64|
            bit < 64 ==> #[trigger] column(with_bit(map, frame), bit, words) == column(map, bit, words)
                + if bit == (frame % 64) as u64 {
                1nat
            } else {
                0nat
            },
{
    assert forall|bit: u64| bit < 64 implies #[trigger] column(with_bit(map, frame), bit, words)
        == column(map, bit, words) + if bit == (frame % 64) as u64 {
        1nat
    } else {
        0nat
    } by {
        lemma_column_update(
            map,
            frame / 64,
            map[frame / 64] | (1u64 << (frame % 64) as u64),
            bit,
            words,
        );
        lemma_set_bit(map[frame / 64], (frame % 64) as u64, bit);
    }
}

pub proof fn lemma_column_without_bit(map: Seq<u64>, frame: int, words: nat)
    requires
        0 <= frame,
        frame / 64 < words == map.len(),
        frame_in(map, frame),
    ensures
        forall|bit: u64|
            bit < 64 ==> #[trigger] column(without_bit(map, frame), bit, words) + (if bit == (frame
                % 64) as u64 {
                1nat
            } else {
                0nat
            }) == column(map, bit, words),
{
    assert forall|bit: u64| bit < 64 implies #[trigger] column(without_bit(map, frame), bit, words)
        + (if bit == (frame % 64) as u64 {
        1nat
    } else {
        0nat
    }) == column(map, bit, words) by {
        lemma_column_update(
            map,
            frame / 64,
            map[frame / 64] & !(1u64 << (frame % 64) as u64),
            bit,
            words,
        );
        lemma_clear_bit(map[frame / 64], (frame % 64) as u64, bit);
    }
}

pub proof fn lemma_column_in_population(map: Seq<u64>, bit: u64, bits: nat, words: nat)
    requires
        bit < bits,
    ensures
        column(map, bit, words) <= population(map, bits, words),
    decreases bits,
{
    if bit < bits - 1 {
        lemma_column_in_population(map, bit, (bits - 1) as nat, words);
    }
}

pub proof fn lemma_strided_term(counts: Seq<usize>, stride: nat, colour: nat, bits: nat, at: int)
    requires
        0 <= at < bits,
        at % stride as int == colour,
        stride > 0,
    ensures
        counts[at] <= strided(counts, stride, colour, bits),
    decreases bits,
{
    if at < bits - 1 {
        lemma_strided_term(counts, stride, colour, (bits - 1) as nat, at);
    }
}

pub proof fn lemma_column_zero(map: Seq<u64>, bit: u64, words: nat)
    requires
        words <= map.len(),
        forall|index: int| 0 <= index < words ==> #[trigger] map[index] == 0,
    ensures
        column(map, bit, words) == 0,
    decreases words,
{
    if words > 0 {
        lemma_column_zero(map, bit, (words - 1) as nat);
        lemma_zero_word(bit);
    }
}

pub proof fn lemma_population_zero(map: Seq<u64>, bits: nat, words: nat)
    requires
        words <= map.len(),
        forall|index: int| 0 <= index < words ==> #[trigger] map[index] == 0,
    ensures
        population(map, bits, words) == 0,
    decreases bits,
{
    if bits > 0 {
        lemma_population_zero(map, (bits - 1) as nat, words);
        lemma_column_zero(map, (bits - 1) as u64, words);
    }
}

pub proof fn lemma_column_congruent(left: Seq<u64>, right: Seq<u64>, bit: u64, words: nat)
    requires
        words <= left.len(),
        words <= right.len(),
        forall|index: int| 0 <= index < words ==> #[trigger] bit_set(left[index], bit) == bit_set(right[index], bit),
    ensures
        column(left, bit, words) == column(right, bit, words),
    decreases words,
{
    if words > 0 {
        lemma_column_congruent(left, right, bit, (words - 1) as nat);
    }
}

pub proof fn lemma_parts_empty(words: usize, managed: Seq<u64>, free: Seq<u64>, by_bit: Seq<usize>)
    requires
        managed.len() == words,
        free.len() == words,
        by_bit.len() == 64,
        forall|index: int| 0 <= index < words ==> #[trigger] managed[index] == 0,
        forall|index: int| 0 <= index < words ==> #[trigger] free[index] == 0,
        forall|bit: int| 0 <= bit < 64 ==> #[trigger] by_bit[bit] == 0,
    ensures
        owned_exactly(words, managed, free, by_bit, 0, 0),
        forall|frame: int| !#[trigger] frame_in(managed, frame),
{
    lemma_population_zero(managed, 64, words as nat);
    assert forall|bit: int| 0 <= bit < 64 implies #[trigger] by_bit[bit] == column(free, bit as u64, words as nat) by {
        lemma_column_zero(free, bit as u64, words as nat);
    }
    assert forall|index: int| 0 <= index < words implies #[trigger] (free[index] & !managed[index]) == 0 by {
        assert(0u64 & !0u64 == 0) by (bit_vector);
    }
    assert forall|frame: int| !#[trigger] frame_in(managed, frame) by {
        if 0 <= frame && frame / 64 < words {
            lemma_zero_word((frame % 64) as u64);
        }
    }
}

pub proof fn lemma_parts_bound(
    words: usize,
    managed: Seq<u64>,
    free: Seq<u64>,
    by_bit: Seq<usize>,
    count: usize,
    limit: usize,
)
    requires
        owned_exactly(words, managed, free, by_bit, count, limit),
    ensures
        count < usize::MAX,
        strided(by_bit, 1, 0, 64) <= count,
        forall|bit: int| 0 <= bit < 64 ==> #[trigger] by_bit[bit] <= count,
{
    let used = domain_words(words);
    assert forall|bit: u64, index: int|
        bit < 64 && used <= index < words implies !#[trigger] bit_set(managed[index], bit) by {
        if bit_set(managed[index], bit) {
            vstd::arithmetic::div_mod::lemma_fundamental_div_mod_converse(
                index * 64 + bit,
                64,
                index,
                bit as int,
            );
            assert(frame_in(managed, index * 64 + bit));
        }
    }
    lemma_population_bound(managed, 64, words as nat, used as nat);
    lemma_strided_columns(by_bit, free, words as nat, 64);
    lemma_population_subset(free, managed, words as nat, 64);
    assert forall|bit: int| 0 <= bit < 64 implies #[trigger] by_bit[bit] <= count by {
        lemma_strided_term(by_bit, 1, 0, 64, bit);
    }
}

pub proof fn lemma_parts_manage(
    words: usize,
    managed: Seq<u64>,
    free: Seq<u64>,
    by_bit: Seq<usize>,
    count: usize,
    limit: usize,
    frame: int,
)
    requires
        owned_exactly(words, managed, free, by_bit, count, limit),
        0 <= frame < frame_count(words),
        !frame_in(managed, frame),
    ensures
        count < usize::MAX,
        frame / 64 < words,
        owned_exactly(
            words,
            with_bit(managed, frame),
            free,
            by_bit,
            (count + 1) as usize,
            if limit < frame + 1 {
                (frame + 1) as usize
            } else {
                limit
            },
        ),
        forall|other: int| #[trigger]
            frame_in(with_bit(managed, frame), other) == (frame_in(managed, other) || other == frame),
{
    lemma_parts_bound(words, managed, free, by_bit, count, limit);
    let after = with_bit(managed, frame);
    lemma_frames_with_bit(managed, frame);
    lemma_column_with_bit(managed, frame, words as nat);
    lemma_population_change(managed, after, words as nat, (frame % 64) as u64, 64);
    assert forall|index: int| 0 <= index < words implies #[trigger] (free[index] & !after[index]) == 0 by {
        if index == frame / 64 {
            lemma_subset_grows(free[index], managed[index], (frame % 64) as u64);
        }
    }
}

pub proof fn lemma_parts_free(
    words: usize,
    managed: Seq<u64>,
    free: Seq<u64>,
    by_bit: Seq<usize>,
    count: usize,
    limit: usize,
    frame: int,
)
    requires
        owned_exactly(words, managed, free, by_bit, count, limit),
        frame_in(managed, frame),
        !frame_in(free, frame),
    ensures
        by_bit[frame % 64] < usize::MAX,
        frame / 64 < words,
        owned_exactly(
            words,
            managed,
            with_bit(free, frame),
            by_bit.update(frame % 64, (by_bit[frame % 64] + 1) as usize),
            count,
            limit,
        ),
        forall|other: int| #[trigger]
            frame_in(with_bit(free, frame), other) == (frame_in(free, other) || other == frame),
{
    lemma_parts_bound(words, managed, free, by_bit, count, limit);
    let after = with_bit(free, frame);
    lemma_frames_with_bit(free, frame);
    lemma_column_with_bit(free, frame, words as nat);
    assert forall|index: int| 0 <= index < words implies #[trigger] (after[index] & !managed[index]) == 0 by {
        if index == frame / 64 {
            lemma_subset_grows(free[index], managed[index], (frame % 64) as u64);
        }
    }
}

pub proof fn lemma_parts_take(
    words: usize,
    managed: Seq<u64>,
    free: Seq<u64>,
    by_bit: Seq<usize>,
    count: usize,
    limit: usize,
    frame: int,
)
    requires
        owned_exactly(words, managed, free, by_bit, count, limit),
        frame_in(free, frame),
    ensures
        by_bit[frame % 64] >= 1,
        frame / 64 < words,
        owned_exactly(
            words,
            managed,
            without_bit(free, frame),
            by_bit.update(frame % 64, (by_bit[frame % 64] - 1) as usize),
            count,
            limit,
        ),
        forall|other: int| #[trigger]
            frame_in(without_bit(free, frame), other) == (frame_in(free, other) && other != frame),
{
    let after = without_bit(free, frame);
    lemma_frames_without_bit(free, frame);
    lemma_column_without_bit(free, frame, words as nat);
    assert(column(after, (frame % 64) as u64, words as nat) + 1 == column(
        free,
        (frame % 64) as u64,
        words as nat,
    ));
    assert forall|index: int| 0 <= index < words implies #[trigger] (after[index] & !managed[index]) == 0 by {
        if index == frame / 64 {
            lemma_subset_grows(free[index], managed[index], (frame % 64) as u64);
        }
    }
}

impl<const WORDS: usize> crate::FrameSpace<WORDS> {
    #[verifier::type_invariant]
    pub(crate) open spec fn ownership(self) -> bool {
        owned_exactly(
            WORDS,
            self.managed@,
            self.free@,
            self.free_by_bit@,
            self.managed_count,
            self.limit,
        )
    }

    pub open(crate) spec fn is_free_frame(self, frame: int) -> bool {
        frame_in(self.free@, frame)
    }

    pub open(crate) spec fn is_managed_frame(self, frame: int) -> bool {
        frame_in(self.managed@, frame)
    }

    pub open(crate) spec fn free_total(self) -> nat {
        strided(self.free_by_bit@, 1, 0, 64)
    }

    // Every frame state with an owner other than free, plus the free ones.
    pub open(crate) spec fn managed_total(self) -> usize {
        self.managed_count
    }

    pub open(crate) spec fn free_of_colour(self, count: usize, colour: int) -> nat {
        if 0 <= colour < count {
            strided(self.free_by_bit@, count as nat, colour as nat, 64)
        } else {
            0
        }
    }

    pub(crate) open spec fn unchanged(self, other: Self) -> bool {
        &&& forall|frame: int| #[trigger] self.is_free_frame(frame) == other.is_free_frame(frame)
        &&& forall|frame: int| #[trigger]
            self.is_managed_frame(frame) == other.is_managed_frame(frame)
        &&& self.free_by_bit@ == other.free_by_bit@
        &&& self.managed_count == other.managed_count
        &&& self.limit == other.limit
        &&& self.hint == other.hint
    }

    // A refusal changes nothing; a success takes exactly the `count` free frames from `first`.
    pub open(crate) spec fn allocated_run(self, before: Self, r: Option<usize>, count: usize) -> bool {
        match r {
            None => self.unchanged(before),
            Some(first) => {
                &&& first + count <= before.limit
                &&& forall|frame: int|
                    first <= frame < first + count ==> #[trigger] before.is_free_frame(frame)
                &&& forall|frame: int|
                    #[trigger] self.is_free_frame(frame) == (before.is_free_frame(frame) && !(first
                        <= frame < first + count))
                &&& forall|frame: int| #[trigger]
                    self.is_managed_frame(frame) == before.is_managed_frame(frame)
                &&& self.limit == before.limit
                &&& self.managed_count == before.managed_count
            },
        }
    }

    pub open(crate) spec fn released_run(self, before: Self, first: int, count: int) -> bool {
        &&& forall|frame: int|
            #[trigger] self.is_free_frame(frame) == (before.is_free_frame(frame) || (first <= frame
                < first + count))
        &&& forall|frame: int| #[trigger]
            self.is_managed_frame(frame) == before.is_managed_frame(frame)
        &&& self.limit == before.limit
    }

    pub(crate) proof fn lemma_free_bounded(self)
        requires
            self.ownership(),
        ensures
            self.free_total() <= self.managed_count,
            self.managed_count < usize::MAX,
            forall|bit: int| 0 <= bit < 64 ==> #[trigger] self.free_by_bit@[bit] <= self.managed_count,
    {
        lemma_parts_bound(
            WORDS,
            self.managed@,
            self.free@,
            self.free_by_bit@,
            self.managed_count,
            self.limit,
        );
    }

    pub(crate) proof fn lemma_managed_bounded(self)
        requires
            self.ownership(),
        ensures
            self.managed_count < usize::MAX,
    {
        self.lemma_free_bounded();
    }

    pub(crate) proof fn lemma_free_is_managed(self, frame: int)
        requires
            self.ownership(),
        ensures
            self.is_free_frame(frame) ==> self.is_managed_frame(frame) && frame < self.limit,
    {
        if self.is_free_frame(frame) {
            lemma_subset_bit(self.free@[frame / 64], self.managed@[frame / 64], (frame % 64) as u64);
            assert(frame_in(self.managed@, frame));
        }
    }

    pub(crate) proof fn lemma_masked_first(self, pattern: u64, from: int)
        requires
            0 <= from,
            from / 64 < WORDS,
        ensures
            forall|bit: u64|
                bit < 64 ==> #[trigger] bit_set(
                    self.free@[from / 64] & pattern & (u64::MAX << (from % 64) as u64),
                    bit,
                ) == (bit_set(self.free@[from / 64], bit) && bit_set(pattern, bit) && (from / 64) * 64 + bit
                    >= from),
    {
        assert forall|bit: u64| bit < 64 implies #[trigger] bit_set(
            self.free@[from / 64] & pattern & (u64::MAX << (from % 64) as u64),
            bit,
        ) == (bit_set(self.free@[from / 64], bit) && bit_set(pattern, bit) && (from / 64) * 64 + bit >= from) by {
            lemma_and_shift(self.free@[from / 64], pattern, (from % 64) as u64, bit);
        }
    }

    pub(crate) proof fn lemma_masked_next(self, pattern: u64, word: int)
        requires
            0 <= word < WORDS,
        ensures
            forall|bit: u64|
                bit < 64 ==> #[trigger] bit_set(self.free@[word] & pattern, bit) == (bit_set(
                    self.free@[word],
                    bit,
                ) && bit_set(pattern, bit)),
    {
        assert forall|bit: u64| bit < 64 implies #[trigger] bit_set(self.free@[word] & pattern, bit) == (
        bit_set(self.free@[word], bit) && bit_set(pattern, bit)) by {
            lemma_and_bit(self.free@[word], pattern, bit);
        }
    }

    pub(crate) proof fn lemma_first_in_word(self, pattern: u64, free: u64, word: int, zeros: int, from: int)
        requires
            0 <= word < WORDS,
            0 <= zeros < 64,
            bit_set(free, zeros as u64),
            forall|bit: u64| bit < zeros ==> !#[trigger] bit_set(free, bit),
            forall|bit: u64|
                bit < 64 ==> #[trigger] bit_set(free, bit) == (bit_set(self.free@[word], bit) && bit_set(
                    pattern,
                    bit,
                ) && word * 64 + bit >= from),
            forall|frame: int|
                from <= frame < word * 64 ==> !(#[trigger] self.is_free_frame(frame) && bit_set(
                    pattern,
                    (frame % 64) as u64,
                )),
        ensures
            from <= word * 64 + zeros,
            self.is_free_frame(word * 64 + zeros),
            bit_set(pattern, ((word * 64 + zeros) % 64) as u64),
            forall|frame: int|
                from <= frame < word * 64 + zeros ==> !(#[trigger] self.is_free_frame(frame) && bit_set(
                    pattern,
                    (frame % 64) as u64,
                )),
    {
        assert(bit_set(free, zeros as u64));
        assert forall|frame: int| from <= frame < word * 64 + zeros implies !(#[trigger] self.is_free_frame(frame) && bit_set(
            pattern,
            (frame % 64) as u64,
        )) by {
            if frame >= word * 64 {
                assert(!bit_set(free, (frame - word * 64) as u64));
            }
        }
    }

    pub(crate) proof fn lemma_empty_word(self, pattern: u64, free: u64, word: int, from: int)
        requires
            0 <= word < WORDS,
            free == 0,
            forall|bit: u64|
                bit < 64 ==> #[trigger] bit_set(free, bit) == (bit_set(self.free@[word], bit) && bit_set(
                    pattern,
                    bit,
                ) && word * 64 + bit >= from),
            forall|frame: int|
                from <= frame < word * 64 ==> !(#[trigger] self.is_free_frame(frame) && bit_set(
                    pattern,
                    (frame % 64) as u64,
                )),
        ensures
            forall|frame: int|
                from <= frame < (word + 1) * 64 ==> !(#[trigger] self.is_free_frame(frame) && bit_set(
                    pattern,
                    (frame % 64) as u64,
                )),
    {
        assert forall|frame: int| from <= frame < (word + 1) * 64 implies !(#[trigger] self.is_free_frame(frame) && bit_set(
            pattern,
            (frame % 64) as u64,
        )) by {
            if frame >= word * 64 {
                lemma_zero_word((frame - word * 64) as u64);
            }
        }
    }

    pub(crate) proof fn lemma_first_taken(self, taken: u64, frame: int, zeros: int)
        requires
            0 <= frame,
            frame / 64 < WORDS,
            taken == (!self.free@[frame / 64]) >> (frame % 64) as u64,
            0 <= zeros < 64,
            bit_set(taken, zeros as u64),
            forall|bit: u64| bit < zeros ==> !#[trigger] bit_set(taken, bit),
        ensures
            frame % 64 + zeros < 64,
            !self.is_free_frame(frame + zeros),
            forall|other: int| frame <= other < frame + zeros ==> #[trigger] self.is_free_frame(other),
    {
        let word = self.free@[frame / 64];
        let offset = (frame % 64) as u64;
        lemma_not_shift(word, offset, zeros as u64);
        assert forall|other: int| frame <= other < frame + zeros implies #[trigger] self.is_free_frame(other) by {
            lemma_not_shift(word, offset, (other - frame) as u64);
            assert(!bit_set(taken, (other - frame) as u64));
        }
    }

    pub(crate) proof fn lemma_rest_free(self, frame: int)
        requires
            0 <= frame,
            frame / 64 < WORDS,
            (!self.free@[frame / 64]) >> (frame % 64) as u64 == 0,
        ensures
            forall|other: int| frame <= other < (frame / 64 + 1) * 64 ==> #[trigger] self.is_free_frame(other),
    {
        let word = self.free@[frame / 64];
        let offset = (frame % 64) as u64;
        assert forall|other: int| frame <= other < (frame / 64 + 1) * 64 implies #[trigger] self.is_free_frame(other) by {
            lemma_not_shift(word, offset, (other - frame) as u64);
            lemma_zero_word((other - frame) as u64);
        }
    }

    pub(crate) proof fn lemma_restored(self, before: Self)
        requires
            before.ownership(),
            self.ownership(),
            forall|frame: int| #[trigger] self.is_free_frame(frame) == before.is_free_frame(frame),
            forall|frame: int| #[trigger]
                self.is_managed_frame(frame) == before.is_managed_frame(frame),
            self.managed_count == before.managed_count,
            self.limit == before.limit,
            self.hint == before.hint,
        ensures
            self.unchanged(before),
    {
        assert forall|bit: int| 0 <= bit < 64 implies #[trigger] self.free_by_bit@[bit] == before.free_by_bit@[bit] by {
            assert forall|index: int| 0 <= index < WORDS implies #[trigger] bit_set(self.free@[index], bit as u64) == bit_set(before.free@[index], bit as u64) by {
                assert(self.is_free_frame(index * 64 + bit) == before.is_free_frame(index * 64 + bit));
            }
            lemma_column_congruent(self.free@, before.free@, bit as u64, WORDS as nat);
        }
        assert(self.free_by_bit@ =~= before.free_by_bit@);
    }

    pub(crate) proof fn lemma_one_taken(self, before: Self, colours: crate::Colours, frame: int)
        requires
            colours.valid(),
            0 <= frame,
            self.free_by_bit@ == before.free_by_bit@.update(
                frame % 64,
                (before.free_by_bit@[frame % 64] - 1) as usize,
            ),
            before.free_by_bit@[frame % 64] >= 1,
            before.free_by_bit@.len() == 64,
        ensures
            forall|colour: int|
                #[trigger] self.free_of_colour(colours.spec_count(), colour) + (if colour
                    == colours.spec_of(frame) {
                    1int
                } else {
                    0int
                }) == before.free_of_colour(colours.spec_count(), colour),
    {
        colours.lemma_colour_of_position(frame);
        assert forall|colour: int|
            #[trigger] self.free_of_colour(colours.spec_count(), colour) + (if colour
                == colours.spec_of(frame) {
                1int
            } else {
                0int
            }) == before.free_of_colour(colours.spec_count(), colour) by {
            if 0 <= colour < colours.spec_count() {
                lemma_strided_change(
                    before.free_by_bit@,
                    self.free_by_bit@,
                    colours.spec_count() as nat,
                    colour as nat,
                    frame % 64,
                    64,
                );
            } else {
                vstd::arithmetic::div_mod::lemma_mod_bound(frame, colours.spec_count() as int);
            }
        }
    }

    pub(crate) proof fn lemma_one_released(self, before: Self, colours: crate::Colours, frame: int)
        requires
            colours.valid(),
            0 <= frame,
            self.free_by_bit@ == before.free_by_bit@.update(
                frame % 64,
                (before.free_by_bit@[frame % 64] + 1) as usize,
            ),
            before.free_by_bit@.len() == 64,
            before.free_by_bit@[frame % 64] < usize::MAX,
        ensures
            forall|colour: int|
                #[trigger] self.free_of_colour(colours.spec_count(), colour) == before.free_of_colour(
                    colours.spec_count(),
                    colour,
                ) + (if colour == colours.spec_of(frame) {
                    1int
                } else {
                    0int
                }),
    {
        colours.lemma_colour_of_position(frame);
        assert forall|colour: int|
            #[trigger] self.free_of_colour(colours.spec_count(), colour) == before.free_of_colour(
                colours.spec_count(),
                colour,
            ) + (if colour == colours.spec_of(frame) {
                1int
            } else {
                0int
            }) by {
            if 0 <= colour < colours.spec_count() {
                lemma_strided_change(
                    before.free_by_bit@,
                    self.free_by_bit@,
                    colours.spec_count() as nat,
                    colour as nat,
                    frame % 64,
                    64,
                );
            } else {
                vstd::arithmetic::div_mod::lemma_mod_bound(frame, colours.spec_count() as int);
            }
        }
    }

    pub(crate) proof fn lemma_free_frame_of_colour(self, colours: crate::Colours, colour: int) -> (frame: int)
        requires
            self.ownership(),
            colours.valid(),
            0 <= colour < colours.spec_count(),
            self.free_of_colour(colours.spec_count(), colour) > 0,
        ensures
            self.is_free_frame(frame),
            colours.spec_of(frame) == colour,
    {
        lemma_strided_member(self.free_by_bit@, colours.spec_count() as nat, colour as nat, 64);
        let bit = choose|bit: int|
            0 <= bit < 64 && bit % colours.spec_count() as int == colour && #[trigger] self.free_by_bit@[bit] > 0;
        lemma_column_member(self.free@, bit as u64, WORDS as nat);
        let index = choose|index: int| 0 <= index < WORDS && bit_set(#[trigger] self.free@[index], bit as u64);
        let frame = index * 64 + bit;
        vstd::arithmetic::div_mod::lemma_fundamental_div_mod_converse(frame, 64, index, bit);
        assert(self.is_free_frame(frame));
        colours.lemma_colour_of_position(frame);
        frame
    }
}

pub proof fn lemma_outside_bit(bits: u64, allowed: u64)
    requires
        bits & !allowed != 0,
    ensures
        exists|bit: u64| bit < 64 && #[trigger] bit_set(bits, bit) && !bit_set(allowed, bit),
{
    let outside = bits & !allowed;
    lemma_lowest_bit(outside);
    let bit = outside.trailing_zeros() as u64;
    assert(bit_set(bits & !allowed, bit) ==> bit_set(bits, bit) && !bit_set(allowed, bit)) by (bit_vector)
        requires
            bit < 64,
    ;
}

pub proof fn lemma_one_is_power()
    ensures
        is_power_of_two(1),
{
    assert(1usize == 1usize << 0u32) by (bit_vector);
}

pub proof fn lemma_zero_word_all()
    ensures
        forall|bit: u64| !#[trigger] bit_set(0, bit),
{
    assert forall|bit: u64| !#[trigger] bit_set(0, bit) by {
        lemma_zero_word(bit);
    }
}

pub proof fn lemma_shift_grows(low: u32, high: u32)
    requires
        low < high < usize::BITS,
    ensures
        1usize << low < 1usize << high,
{
    assert(1usize << low < 1usize << high) by (bit_vector)
        requires
            low < high < usize::BITS,
    ;
}

pub proof fn lemma_power_positive(value: usize)
    requires
        is_power_of_two(value),
    ensures
        value >= 1,
{
    let shift = choose|shift: u32| shift < usize::BITS && value == 1usize << shift;
    assert(shift < usize::BITS ==> (1usize << shift) >= 1) by (bit_vector);
}

pub proof fn lemma_strided_below(counts: Seq<usize>, stride: nat, colour: nat, bits: nat)
    requires
        stride > 0,
        bits <= colour < stride,
    ensures
        strided(counts, stride, colour, bits) == 0,
    decreases bits,
{
    if bits > 0 {
        lemma_strided_below(counts, stride, colour, (bits - 1) as nat);
        vstd::arithmetic::div_mod::lemma_small_mod((bits - 1) as nat, stride);
    }
}

} // verus!
