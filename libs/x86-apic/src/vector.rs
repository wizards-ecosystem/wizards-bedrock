//! Assignment of kernel-owned interrupt vectors.

const FIRST_PIC_VECTOR: u8 = 0x20;

/// First vector outside the CPU and remapped-8259 ranges.
pub const FIRST_ASSIGNABLE: u8 = 0x30;

/// A vector available to one kernel-owned interrupt source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct InterruptVector(u8);

impl InterruptVector {
    /// Admit a vector outside the CPU and remapped-8259 ranges.
    pub const fn new(vector: u8) -> Option<Self> {
        if vector < FIRST_PIC_VECTOR {
            return None;
        }
        if vector >= FIRST_PIC_VECTOR && vector < FIRST_ASSIGNABLE {
            return None;
        }
        Some(Self(vector))
    }

    /// The number written to interrupt-controller and descriptor-table fields.
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// One duplicate-free set of interrupt-source assignments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assignment<const N: usize>([InterruptVector; N]);

impl<const N: usize> Assignment<N> {
    /// Validate every source boundary and every pair before assigning either.
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

    /// Consume the set into its source-ordered values.
    pub const fn into_array(self) -> [InterruptVector; N] {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_exhausts_all_256_vectors() {
        for raw in u8::MIN..=u8::MAX {
            assert_eq!(
                InterruptVector::new(raw).map(InterruptVector::get),
                (raw >= 0x30).then_some(raw),
                "vector {raw:#x}"
            );
        }
    }

    #[test]
    fn every_cpu_vector_is_reserved() {
        for reserved in u8::MIN..FIRST_PIC_VECTOR {
            assert!(
                Assignment::new([0x30, reserved, 0xfe, 0xff]).is_none(),
                "reserved vector {reserved:#x}"
            );
        }
    }

    #[test]
    fn every_remapped_pic_vector_is_reserved() {
        for reserved in FIRST_PIC_VECTOR..FIRST_ASSIGNABLE {
            assert!(
                Assignment::new([0x30, reserved, 0xfe, 0xff]).is_none(),
                "reserved vector {reserved:#x}"
            );
        }
    }

    #[test]
    fn every_pair_is_accepted_exactly_when_assignable_and_distinct() {
        for first in u8::MIN..=u8::MAX {
            for second in u8::MIN..=u8::MAX {
                let expected = first >= 0x30 && second >= 0x30 && first != second;
                assert_eq!(
                    Assignment::new([first, second]).is_some(),
                    expected,
                    "vectors {first:#x}, {second:#x}"
                );
            }
        }
    }

    #[test]
    fn a_complete_assignment_preserves_source_order() {
        let assigned = Assignment::new([0xff, 0xfe, 0x40, 0x41, 0x80, 0x42])
            .expect("valid vector assignment")
            .into_array();
        assert_eq!(
            assigned.map(InterruptVector::get),
            [0xff, 0xfe, 0x40, 0x41, 0x80, 0x42]
        );
    }
}
