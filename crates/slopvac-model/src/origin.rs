//! Where a finding sits inside a container document: a segment of extracted text.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Longest locator, in code points.
const LOCATOR_MAX_CHARS: usize = 512;
/// Longest label, in code points.
const LABEL_MAX_CHARS: usize = 256;
/// Longest segment id, in characters.
const SEGMENT_ID_MAX_CHARS: usize = 64;

/// A character the contract never allows in a locator or label: a control, a line or
/// paragraph separator, or a bidirectional override.
fn forbidden(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Why an origin cannot exist.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum OriginError {
    /// A locator is empty, too long, has a forbidden character or a bad `%` escape.
    #[error("invalid origin locator: {0}")]
    Locator(&'static str),
    /// A label is empty, too long or has a forbidden character.
    #[error("invalid origin label: {0}")]
    Label(&'static str),
    /// A segment id is not 1 to 64 characters of `[A-Za-z0-9._-]` starting with a letter or digit.
    #[error("invalid segment id")]
    SegmentId,
    /// A range starts after it ends.
    #[error("segment range start {start} is after end {end}")]
    Range {
        /// Start offset.
        start: usize,
        /// End offset.
        end: usize,
    },
}

fn check_locator(s: &str) -> Result<(), OriginError> {
    if s.is_empty() {
        return Err(OriginError::Locator("empty"));
    }
    if s.chars().count() > LOCATOR_MAX_CHARS {
        return Err(OriginError::Locator("too long"));
    }
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if forbidden(c) {
            return Err(OriginError::Locator(
                "forbidden character, percent-escape it",
            ));
        }
        if c == '%' {
            let ok = (0..2).all(|_| {
                chars
                    .next()
                    .is_some_and(|h| h.is_ascii_digit() || ('A'..='F').contains(&h))
            });
            if !ok {
                return Err(OriginError::Locator(
                    "`%` must start %XX with uppercase hex",
                ));
            }
        }
    }
    Ok(())
}

fn check_label(s: &str) -> Result<(), OriginError> {
    if s.is_empty() {
        Err(OriginError::Label("empty"))
    } else if s.chars().count() > LABEL_MAX_CHARS {
        Err(OriginError::Label("too long"))
    } else if s.chars().any(forbidden) {
        Err(OriginError::Label("forbidden character"))
    } else {
        Ok(())
    }
}

fn check_segment(s: &str) -> Result<(), OriginError> {
    let mut chars = s.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    if first_ok
        && s.len() <= SEGMENT_ID_MAX_CHARS
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        Ok(())
    } else {
        Err(OriginError::SegmentId)
    }
}

/// Range of UTF-8 bytes in a segment's text, `[start, end)`, local to that segment.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(try_from = "SegmentRangeRepr")]
#[schemars(with = "SegmentRangeRepr")]
pub struct SegmentRange {
    start: usize,
    end: usize,
}

/// Wire shape of [`SegmentRange`].
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "SegmentRange", extend("additionalProperties" = false))]
struct SegmentRangeRepr {
    start: usize,
    end: usize,
}

impl TryFrom<SegmentRangeRepr> for SegmentRange {
    type Error = OriginError;
    fn try_from(r: SegmentRangeRepr) -> Result<Self, OriginError> {
        Self::new(r.start, r.end)
    }
}

impl SegmentRange {
    /// Build a range, rejecting `start > end`.
    pub const fn new(start: usize, end: usize) -> Result<Self, OriginError> {
        if start > end {
            Err(OriginError::Range { start, end })
        } else {
            Ok(Self { start, end })
        }
    }

    /// First byte.
    #[must_use]
    pub const fn start(&self) -> usize {
        self.start
    }

    /// First byte after the range.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.end
    }
}

macro_rules! checked_string {
    ($(#[$meta:meta])* $name:ident, $check:path, $schema:expr) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Validate and wrap the text.
            pub fn new(value: impl Into<String>) -> Result<Self, OriginError> {
                let value = value.into();
                $check(&value)?;
                Ok(Self(value))
            }

            /// The text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = OriginError;
            fn try_from(value: String) -> Result<Self, OriginError> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                $schema
            }
        }
    };
}

checked_string!(
    /// Percent-escaped locator of a segment, unique within its document.
    OriginLocator,
    check_locator,
    json_schema!({
        "type": "string",
        "minLength": 1,
        "maxLength": 512,
        "pattern": "^(?:[^%\\u0000-\\u001F\\u007F-\\u009F\\u2028\\u2029\\u202A-\\u202E\\u2066-\\u2069]|%[0-9A-F]{2})+$"
    })
);
checked_string!(
    /// Human-readable name of a segment, for display only.
    OriginLabel,
    check_label,
    json_schema!({
        "type": "string",
        "minLength": 1,
        "maxLength": 256,
        "pattern": "^[^\\u0000-\\u001F\\u007F-\\u009F\\u2028\\u2029\\u202A-\\u202E\\u2066-\\u2069]+$"
    })
);
checked_string!(
    /// Identifier of a segment, unique within its document.
    SegmentId,
    check_segment,
    json_schema!({
        "type": "string",
        "pattern": "^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"
    })
);

/// Where a finding sits in a container document such as a Word file or a diagram.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "OriginWire", into = "OriginWire")]
#[schemars(with = "OriginWire")]
pub struct Origin {
    locator: OriginLocator,
    label: OriginLabel,
    segment: SegmentId,
    range: SegmentRange,
}

/// Wire shape of [`Origin`].
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(rename = "Origin", extend("additionalProperties" = false, "patternProperties" = {"^x-": true}))]
struct OriginWire {
    /// Percent-escaped locator, unique per segment within the document.
    locator: OriginLocator,
    /// Display name of the segment.
    label: OriginLabel,
    /// Segment id, unique within the document.
    segment: SegmentId,
    /// Extracted match range in UTF-8 bytes, local to the segment's text.
    range: SegmentRange,
}

impl TryFrom<OriginWire> for Origin {
    type Error = OriginError;
    fn try_from(w: OriginWire) -> Result<Self, OriginError> {
        Ok(Self {
            locator: w.locator,
            label: w.label,
            segment: w.segment,
            range: w.range,
        })
    }
}

impl From<Origin> for OriginWire {
    fn from(o: Origin) -> Self {
        Self {
            locator: o.locator,
            label: o.label,
            segment: o.segment,
            range: o.range,
        }
    }
}

impl Origin {
    /// Build an origin from already validated parts.
    #[must_use]
    pub const fn new(
        locator: OriginLocator,
        label: OriginLabel,
        segment: SegmentId,
        range: SegmentRange,
    ) -> Self {
        Self {
            locator,
            label,
            segment,
            range,
        }
    }

    /// Percent-escaped locator.
    #[must_use]
    pub const fn locator(&self) -> &OriginLocator {
        &self.locator
    }

    /// Display label.
    #[must_use]
    pub const fn label(&self) -> &OriginLabel {
        &self.label
    }

    /// Segment id.
    #[must_use]
    pub const fn segment(&self) -> &SegmentId {
        &self.segment
    }

    /// Match range local to the segment.
    #[must_use]
    pub const fn range(&self) -> SegmentRange {
        self.range
    }
}
