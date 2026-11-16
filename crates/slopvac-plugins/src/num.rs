//! Integer conversions shared across the crate.

/// Widen a `usize` to `u64` for comparison with a byte limit.
///
/// `usize` is at most 64 bits on every supported target, so the conversion is exact.
/// If a wider target ever appears, the value saturates, which keeps every limit check
/// on the refusing side.
pub(crate) fn widen(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::widen;

    #[test]
    fn widen_keeps_the_value() {
        assert_eq!(widen(0), 0);
        assert_eq!(widen(65_536), 65_536);
    }
}
