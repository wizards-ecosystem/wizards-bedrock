#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Progress {
    Done,
    Pending,
}

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

impl ProcessStep {
    #[must_use]
    pub fn next(self, progress: Progress) -> Self {
        if progress == Progress::Pending {
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

impl WorldStep {
    #[must_use]
    pub fn next(self, progress: Progress) -> Self {
        if progress == Progress::Pending {
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
