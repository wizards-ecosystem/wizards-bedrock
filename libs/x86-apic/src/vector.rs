const FIRST_PIC_VECTOR: u8 = 0x20;

const FIRST_ASSIGNABLE: u8 = 0x30;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct InterruptVector(u8);

impl InterruptVector {
    pub const fn new(vector: u8) -> Option<Self> {
        if vector < FIRST_PIC_VECTOR {
            return None;
        }
        if vector >= FIRST_PIC_VECTOR && vector < FIRST_ASSIGNABLE {
            return None;
        }
        Some(Self(vector))
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Assignment<const N: usize>([InterruptVector; N]);

impl<const N: usize> Assignment<N> {
    pub const fn new(vectors: [u8; N]) -> Option<Self> {
        let mut assigned = [InterruptVector(FIRST_ASSIGNABLE); N];
        let mut index = 0;
        while index < N {
            let Some(vector) = InterruptVector::new(vectors[index]) else {
                return None;
            };
            let mut other = 0;
            while other < index {
                if assigned[other].get() == vector.get() {
                    return None;
                }
                other += 1;
            }
            assigned[index] = vector;
            index += 1;
        }
        Some(Self(assigned))
    }

    pub const fn into_array(self) -> [InterruptVector; N] {
        self.0
    }
}
