//! Source-defined ACPI authority shared by the qualified machine generators.

use acpi_tables::{DmaHandoff, Sdt};

pub(crate) const DMA_HANDOFF_FILE: &str = "dma-handoff.bin";

pub(crate) fn dma_handoff_table() -> [u8; DmaHandoff::LEN] {
    let mut table = [0u8; DmaHandoff::LEN];
    table[0..4].copy_from_slice(&Sdt::DMA_HANDOFF);
    table[4..8].copy_from_slice(&(DmaHandoff::LEN as u32).to_le_bytes());
    table[8] = 1;
    table[10..16].copy_from_slice(b"WIZARD");
    table[16..24].copy_from_slice(b"BEDROCK ");
    table[24..28].copy_from_slice(&1u32.to_le_bytes());
    table[28..32].copy_from_slice(b"WZRD");
    table[32..36].copy_from_slice(&1u32.to_le_bytes());
    table[Sdt::LEN..Sdt::LEN + 2].copy_from_slice(&0u16.to_le_bytes());
    table[Sdt::LEN + 2..Sdt::LEN + 4].copy_from_slice(&DmaHandoff::REQUIRED_FLAGS.to_le_bytes());
    table[9] = 0u8.wrapping_sub(table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_dma_handoff_table_carries_the_complete_contract() {
        let table = dma_handoff_table();
        let handoff = DmaHandoff::new(&table).expect("generated DMA handoff authority");
        assert_eq!(handoff.segment(), 0);
        assert_eq!(
            table.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)),
            0
        );
    }
}
