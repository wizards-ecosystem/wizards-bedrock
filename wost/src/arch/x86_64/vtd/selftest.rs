use super::*;

impl Vtd {
    /// Inject a bounded invalidation refusal for the machine ownership test.
    #[cfg(feature = "selftest")]
    pub fn force_invalidation_failures_for_test(&mut self, count: u8) {
        self.forced_invalidation_failures = count;
    }

    /// Number of read-and-write-drained IOTLB invalidations completed by this
    /// owner, including the takeover drain and every live revocation drain.
    #[cfg(feature = "selftest")]
    pub const fn completed_dma_drains_for_test(&self) -> u64 {
        self.completed_dma_drains
    }

    /// Whether the unit's root-table pointer names this domain's root table.
    #[cfg(feature = "selftest")]
    pub fn walks_kernel_root_for_test(&self) -> Option<bool> {
        let pointer = read_u64(&self.registers, vtd::RTADDR).ok()?;
        Some(root_table_address(pointer) == self.root.address())
    }

    /// Every page the second-level tree the unit walks reaches, read back from the table frames.
    #[cfg(feature = "selftest")]
    pub fn reachable_pages_for_test(&self) -> Option<Vec<u64>> {
        let mut pages = Vec::new();
        self.collect_reachable(0, self.width.levels() - 1, &mut pages)?;
        Some(pages)
    }

    #[cfg(feature = "selftest")]
    fn collect_reachable(&self, table: usize, level: u8, pages: &mut Vec<u64>) -> Option<()> {
        for slot in 0..ENTRIES {
            let entry = self.tables.read_u64(table * ENTRIES + slot)?;
            if !second_level::is_present(entry) {
                continue;
            }
            if level == 0 {
                pages.push(second_level::address(entry));
            } else {
                self.collect_reachable(self.table_index(entry)?, level - 1, pages)?;
            }
        }
        Some(())
    }
}
