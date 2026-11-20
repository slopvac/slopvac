//! Parameter values: plain strings and numbers, never domain types.

/// A parameter value, ready for the template.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Value {
    /// Plain text.
    Text(String),
    /// A count.
    Count(u64),
    /// A path in the form the user sees.
    Path(String),
}

/// A named parameter value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Arg {
    /// The parameter name.
    pub name: &'static str,
    /// The value.
    pub value: Value,
}

impl Arg {
    /// Pair a name with a value.
    #[must_use]
    pub const fn new(name: &'static str, value: Value) -> Self {
        Self { name, value }
    }
}

/// A number that can be passed as a `#[count]` parameter.
pub trait ToCount {
    /// The count as a `u64`.
    fn to_count(&self) -> u64;
}

macro_rules! to_count {
    ($($ty:ty),*) => {$(
        impl ToCount for $ty {
            fn to_count(&self) -> u64 {
                u64::from(*self)
            }
        }
    )*};
}

to_count!(u8, u16, u32, u64);

impl ToCount for usize {
    fn to_count(&self) -> u64 {
        u64::try_from(*self).unwrap_or(u64::MAX)
    }
}
