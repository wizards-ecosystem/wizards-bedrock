use vstd::prelude::{verus_spec, verus_verify};

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Progress {
    Done,
    Pending,
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProcessStep {
    Exit,
    Quiesce,
    AbandonQueue,
    DrainTable,
    ClearSpace,
    ReleaseRecords,
    Released,
}

#[verus_verify]
impl ProcessStep {
    #[must_use]
    #[verus_spec(r =>
        ensures
            crate::proof::each_step_moves_toward_the_end(
                crate::proof::process_steps_left(self),
                crate::proof::process_steps_left(r),
                progress,
            ),
    )]
    pub fn next(self, progress: Progress) -> Self {
        if matches!(progress, Progress::Pending) {
            return self;
        }
        match self {
            Self::Exit => Self::Quiesce,
            Self::Quiesce => Self::AbandonQueue,
            Self::AbandonQueue => Self::DrainTable,
            Self::DrainTable => Self::ClearSpace,
            Self::ClearSpace => Self::ReleaseRecords,
            Self::ReleaseRecords | Self::Released => Self::Released,
        }
    }
}

#[verus_verify]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WorldStep {
    Kill,
    ExitProcesses,
    Quiesce,
    AbandonQueuesAndTables,
    DestroyObjects,
    ClearSpaces,
    RevokeMappers,
    ClosePins,
    Detach,
    Dissolve,
    Reclaim,
    Finished,
}

#[verus_verify]
impl WorldStep {
    #[must_use]
    #[verus_spec(r =>
        ensures
            crate::proof::each_step_moves_toward_the_end(
                crate::proof::world_steps_left(self),
                crate::proof::world_steps_left(r),
                progress,
            ),
    )]
    pub fn next(self, progress: Progress) -> Self {
        if matches!(progress, Progress::Pending) {
            return self;
        }
        match self {
            Self::Kill => Self::ExitProcesses,
            Self::ExitProcesses => Self::Quiesce,
            Self::Quiesce => Self::AbandonQueuesAndTables,
            Self::AbandonQueuesAndTables => Self::DestroyObjects,
            Self::DestroyObjects => Self::ClearSpaces,
            Self::ClearSpaces => Self::RevokeMappers,
            Self::RevokeMappers => Self::ClosePins,
            Self::ClosePins => Self::Detach,
            Self::Detach => Self::Dissolve,
            Self::Dissolve => Self::Reclaim,
            Self::Reclaim | Self::Finished => Self::Finished,
        }
    }
}
