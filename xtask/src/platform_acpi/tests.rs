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
