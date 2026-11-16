use super::{Flags, MapError, ThreadContext, UserMapError, mm, paging, with_active_space};

const FRAME_SIZE: u64 = 4096;

pub(crate) struct ActiveMapping {
    base: u64,
    len: u64,
}

pub(crate) fn map_user_page_for_test(
    context: &mut ThreadContext,
    base: u64,
    run: mm::Segment,
    flags: Flags,
) -> Option<Result<ActiveMapping, UserMapError>> {
    if run.len() != FRAME_SIZE {
        return None;
    }
    with_active_space(context, |space| space.map_user(base, run.into(), flags)).map(|result| {
        result.map(|()| ActiveMapping {
            base,
            len: FRAME_SIZE,
        })
    })
}

pub(crate) fn unmap_for_test(
    context: &mut ThreadContext,
    mapping: ActiveMapping,
) -> Option<Result<paging::Unmapped, MapError>> {
    with_active_space(context, |space| space.unmap(mapping.base, mapping.len))
}
