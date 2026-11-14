use super::*;

#[test]
fn a_gigantic_weight_still_pays() {
    let mut entity = Entity::new(u32::MAX);
    entity.charge(1);
    assert_eq!(entity.weight, MAX_WEIGHT);
    assert_eq!(entity.vruntime(), 1, "the clock must never stand still");
}
