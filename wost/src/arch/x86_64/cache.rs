use core::arch::x86_64::{__cpuid_count, __get_cpuid_max};

use x86_cpuid::cache::{
    CACHE_LEAF, Cache, EXTENDED_CACHE_LEAF, EXTENDED_FEATURE_LEAF, colour_count,
    has_extended_cache_leaf,
};

// Real parts describe a handful of caches; the bound stops a hypervisor that never answers zero.
const SUB_LEAVES: u32 = 16;

pub(crate) fn colours() -> u32 {
    let (basic, _) = __get_cpuid_max(0);
    let (extended, _) = __get_cpuid_max(0x8000_0000);
    let described = |leaf| Cache::from_leaf(__cpuid_count(leaf, 0).eax, 0, 0).is_some();
    let leaf = if basic >= CACHE_LEAF && described(CACHE_LEAF) {
        CACHE_LEAF
    } else if has_extended_cache_leaf(extended, __cpuid_count(EXTENDED_FEATURE_LEAF, 0).ecx) {
        EXTENDED_CACHE_LEAF
    } else {
        return 1;
    };
    colour_count((0..SUB_LEAVES).map_while(|sub_leaf| {
        let answer = __cpuid_count(leaf, sub_leaf);
        Cache::from_leaf(answer.eax, answer.ebx, answer.ecx)
    }))
}
