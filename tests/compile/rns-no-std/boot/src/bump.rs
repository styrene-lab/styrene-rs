//! Checked address arithmetic shared by the allocator and standalone host tests.
#![forbid(unsafe_code)]

/// Reserve a nonempty aligned range, without changing state on failure.
/// `end` is exclusive. Zero-sized requests are rejected (GlobalAlloc's contract
/// excludes them). The caller owns backing storage and never resets the cursor.
pub fn reserve(cursor: usize, end: usize, size: usize, align: usize) -> Option<(usize, usize)> {
    if size == 0 || !align.is_power_of_two() || cursor > end {
        return None;
    }
    let start = cursor.checked_add(align - 1)? & !(align - 1);
    let next = start.checked_add(size)?;
    (next <= end).then_some((start, next))
}

#[cfg(test)]
mod tests {
    use super::reserve;

    #[test]
    fn alignment_and_exact_fit() {
        for align in [1, 2, 16, 4096, 65536] {
            let (start, next) = reserve(0x10001, 0x40000, 7, align).unwrap();
            assert_eq!(start % align, 0);
            assert_eq!(next, start + 7);
        }
        assert_eq!(reserve(0x1000, 0x1010, 16, 16), Some((0x1000, 0x1010)));
        assert_eq!(reserve(0x1010, 0x1010, 1, 1), None);
    }

    #[test]
    fn invalid_overflow_and_exhaustion() {
        for (cursor, end, size, align) in [
            (1, 2, 0, 1),
            (1, 2, 1, 0),
            (1, 8, 1, 3),
            (9, 8, 1, 1),
            (usize::MAX, usize::MAX, 1, 16),
            (usize::MAX - 7, usize::MAX, 16, 1),
            (1, 16, 16, 16),
        ] {
            assert_eq!(reserve(cursor, end, size, align), None);
        }
    }

    #[test]
    fn live_allocations_never_overlap() {
        let mut cursor = 0x1003;
        let end = 0x2000;
        let mut ranges = std::vec::Vec::new();
        for index in 0..256 {
            let align = 1 << (index % 7);
            if let Some((start, next)) = reserve(cursor, end, index % 31 + 1, align) {
                assert!(start >= cursor && next <= end);
                assert_eq!(start % align, 0);
                for &(old_start, old_end) in &ranges {
                    assert!(next <= old_start || start >= old_end);
                }
                ranges.push((start, next));
                cursor = next;
            }
        }
        assert!(ranges.len() > 20);
    }
}
