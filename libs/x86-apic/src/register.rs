use crate::Register;

pub const ID: Register = Register::new(0x020);

pub const VERSION: Register = Register::new(0x030);

pub const TASK_PRIORITY: Register = Register::new(0x080);

pub const IN_SERVICE: [Register; 8] = [
    Register::new(0x100),
    Register::new(0x110),
    Register::new(0x120),
    Register::new(0x130),
    Register::new(0x140),
    Register::new(0x150),
    Register::new(0x160),
    Register::new(0x170),
];

pub const INTERRUPT_REQUEST: [Register; 8] = [
    Register::new(0x200),
    Register::new(0x210),
    Register::new(0x220),
    Register::new(0x230),
    Register::new(0x240),
    Register::new(0x250),
    Register::new(0x260),
    Register::new(0x270),
];

pub const END_OF_INTERRUPT: Register = Register::new(0x0B0);

pub const SPURIOUS_INTERRUPT_VECTOR: Register = Register::new(0x0F0);

// Latches on write, so it is written before it is read.
pub const ERROR_STATUS: Register = Register::new(0x280);

pub const COMMAND: Register = Register::new(0x300);

const LVT_CMCI: Register = Register::new(0x2F0);

pub const LVT_TIMER: Register = Register::new(0x320);

const LVT_THERMAL: Register = Register::new(0x330);

const LVT_PERFORMANCE: Register = Register::new(0x340);

const LVT_LINT0: Register = Register::new(0x350);

const LVT_LINT1: Register = Register::new(0x360);

pub const LVT_ERROR: Register = Register::new(0x370);

pub const LOCAL_VECTOR_TABLE: [Register; 7] = [
    LVT_TIMER,
    LVT_THERMAL,
    LVT_PERFORMANCE,
    LVT_LINT0,
    LVT_LINT1,
    LVT_ERROR,
    LVT_CMCI,
];

pub const INITIAL_COUNT: Register = Register::new(0x380);

pub const CURRENT_COUNT: Register = Register::new(0x390);

pub const DIVIDE_CONFIGURATION: Register = Register::new(0x3E0);
