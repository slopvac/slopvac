//! Integer conversions shared across the crate.

/// Widen a `usize` to `u64` for comparison with a byte limit.
///
/// `usize` is at most 64 bits on every supported target, so the conversion is exact.
/// If a wider target ever appears, the value saturates, which keeps every limit check
/// on the refusing side.
pub(crate) fn widen(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Append formatted text to a `String`.
///
/// `String`'s `fmt::Write` never returns an error, so the result carries no
/// information. This is the one place that discards it.
pub(crate) fn push_fmt(out: &mut String, args: std::fmt::Arguments<'_>) {
    use std::fmt::Write as _;
    #[expect(
        clippy::let_underscore_must_use,
        reason = "`String`'s `fmt::Write` never returns an error"
    )]
    let _ = out.write_fmt(args);
}

#[cfg(test)]
mod tests {
    use super::{push_fmt, widen};

    #[test]
    fn push_fmt_appends_to_the_string() {
        let mut s = String::from("a");
        push_fmt(&mut s, format_args!("{}-{:02X}", "b", 10));
        assert_eq!(s, "ab-0A");
    }

    #[test]
    fn widen_keeps_the_value() {
        assert_eq!(widen(0), 0);
        assert_eq!(widen(65_536), 65_536);
    }
}
