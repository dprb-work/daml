//! Lossless admission for DAML's bounded ISO extended-calendar profile.
use crate::{Span, StringValue};
use std::{borrow::Cow, error::Error, fmt, fmt::Write as _};

/// An intrinsic temporal scalar kind, independent of assertion tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TemporalKind {
    /// A complete proleptic Gregorian calendar date.
    Date,
    /// A clock label with an optional explicit offset.
    Time,
    /// A calendar date and clock label with an optional explicit offset.
    DateTime,
}
impl TemporalKind {
    /// Returns the stable semantic transport spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Date => "date",
            Self::Time => "time",
            Self::DateTime => "datetime",
        }
    }
    pub(crate) fn constructor(self) -> &'static str {
        match self {
            Self::Date => "Date",
            Self::Time => "Time",
            Self::DateTime => "DateTime",
        }
    }
    pub(crate) fn from_constructor(name: &str) -> Option<Self> {
        match name {
            "Date" => Some(Self::Date),
            "Time" => Some(Self::Time),
            "DateTime" => Some(Self::DateTime),
            _ => None,
        }
    }
    pub(crate) fn from_str(name: &str) -> Option<Self> {
        match name {
            "date" => Some(Self::Date),
            "time" => Some(Self::Time),
            "datetime" => Some(Self::DateTime),
            _ => None,
        }
    }
}

/// An invalid ISO-profile value. No host clock, timezone or leap schedule is consulted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TemporalError(&'static str);
impl fmt::Display for TemporalError {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(self.0)
    }
}
impl Error for TemporalError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TemporalMetadata {
    pub(crate) kind: TemporalKind,
    pub(crate) offset: Option<i16>,
}
impl TemporalMetadata {
    pub(crate) fn value(self, text: &str) -> TemporalValue<'_> {
        TemporalValue {
            text: Cow::Borrowed(text),
            metadata: self,
        }
    }
    pub(crate) fn canonical(kind: TemporalKind, text: &str) -> Self {
        let offset = if kind == TemporalKind::Date {
            None
        } else if text.ends_with('Z') {
            Some(0)
        } else {
            let start = text.len().saturating_sub(6);
            let tail = &text.as_bytes()[start..];
            if tail.len() == 6 && matches!(tail[0], b'+' | b'-') {
                let magnitude = i16::from((tail[1] - b'0') * 10 + tail[2] - b'0') * 60
                    + i16::from((tail[4] - b'0') * 10 + tail[5] - b'0');
                Some(if tail[0] == b'-' {
                    -magnitude
                } else {
                    magnitude
                })
            } else {
                None
            }
        };
        Self { kind, offset }
    }
}

/// An admitted temporal scalar, borrowing already canonical text when possible.
#[derive(Debug, Clone)]
pub struct TemporalValue<'a> {
    text: Cow<'a, str>,
    metadata: TemporalMetadata,
}
impl<'a> TemporalValue<'a> {
    /// Admits the documented ISO profile and canonicalizes only when necessary.
    ///
    /// Omitted lower clock fields become zero; fractional digits and exceptional
    /// labels are retained. An absent offset never becomes UTC.
    pub fn parse(kind: TemporalKind, text: impl Into<Cow<'a, str>>) -> Result<Self, TemporalError> {
        let text = text.into();
        let parsed = admit(kind, text.as_ref())?;
        let text = if parsed.canonical {
            text
        } else {
            Cow::Owned(parsed.write(text.as_ref()))
        };
        Ok(Self {
            text,
            metadata: TemporalMetadata {
                kind,
                offset: parsed.offset,
            },
        })
    }
    /// Returns the intrinsic kind.
    pub fn kind(&self) -> TemporalKind {
        self.metadata.kind
    }
    /// Returns canonical ISO text with every supplied fractional digit.
    pub fn as_str(&self) -> &str {
        self.text.as_ref()
    }
    /// Returns the explicit offset in minutes; `None` remains distinct from UTC.
    pub fn offset_minutes(&self) -> Option<i16> {
        self.metadata.offset
    }
    /// Borrows this admitted value without allocating or revalidating.
    pub fn as_borrowed(&self) -> TemporalValue<'_> {
        self.metadata.value(self.as_str())
    }
    /// Retains this admitted value independently, moving owned text when available.
    pub fn into_owned(self) -> TemporalValue<'static> {
        TemporalValue {
            text: Cow::Owned(self.text.into_owned()),
            metadata: self.metadata,
        }
    }
    pub(crate) fn into_text(self) -> Cow<'a, str> {
        self.text
    }
}
impl PartialEq for TemporalValue<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.metadata == other.metadata
            && equality_parts(self.as_str()) == equality_parts(other.as_str())
    }
}
impl Eq for TemporalValue<'_> {}
fn equality_parts(text: &str) -> (&str, &str, &str) {
    let Some(dot) = text.find('.') else {
        let end = text
            .find('Z')
            .or_else(|| {
                text.rfind(['+', '-'])
                    .filter(|&i| i > text.find('T').unwrap_or(0) + 2)
            })
            .unwrap_or(text.len());
        return (&text[..end], "", &text[end..]);
    };
    let end = text[dot + 1..]
        .find(['Z', '+', '-'])
        .map_or(text.len(), |i| dot + 1 + i);
    (
        &text[..dot],
        text[dot + 1..end].trim_end_matches('0'),
        &text[end..],
    )
}

// Only the retained source allocation may lend bytes to an admitted payload.
// Store an address for identity comparison, never for dereferencing. Moving a
// String keeps its allocation; public document/definition access cannot mutate it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceIdentity {
    address: usize,
    len: usize,
}
impl SourceIdentity {
    fn new(source: &str) -> Self {
        Self {
            address: source.as_ptr() as usize,
            len: source.len(),
        }
    }
}

/// An admitted source-backed temporal payload. Its private fields prevent unchecked construction.
///
/// Access is bound to the retained source allocation, including when canonical
/// text is owned. Moving the source owner preserves this binding. Equality
/// compares the parsed representation and metadata, not the allocation identity.
#[derive(Debug)]
pub struct ParsedTemporal {
    text: StringValue,
    metadata: TemporalMetadata,
    source: SourceIdentity,
}
impl PartialEq for ParsedTemporal {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.metadata == other.metadata
    }
}
impl ParsedTemporal {
    pub(crate) fn parse(
        kind: TemporalKind,
        text: StringValue,
        source: &str,
    ) -> Result<Self, TemporalError> {
        let source_identity = SourceIdentity::new(source);
        match text {
            StringValue::Source(span) => {
                let admitted = TemporalValue::parse(kind, &source[span.start..span.end])?;
                let metadata = admitted.metadata;
                let text = match admitted.text {
                    Cow::Borrowed(_) => StringValue::Source(span),
                    Cow::Owned(text) => StringValue::Decoded(text),
                };
                Ok(Self {
                    text,
                    metadata,
                    source: source_identity,
                })
            }
            StringValue::Decoded(text) => {
                let admitted = TemporalValue::parse(kind, text)?;
                Ok(Self {
                    metadata: admitted.metadata,
                    text: StringValue::Decoded(admitted.text.into_owned()),
                    source: source_identity,
                })
            }
        }
    }
    pub(crate) fn value<'a>(&'a self, source: &'a str) -> Option<TemporalValue<'a>> {
        if self.source != SourceIdentity::new(source) {
            return None;
        }
        let text = match &self.text {
            StringValue::Source(Span { start, end }) => source.get(*start..*end)?,
            StringValue::Decoded(text) => text,
        };
        Some(self.metadata.value(text))
    }
}

struct Admitted {
    canonical: bool,
    date_end: usize,
    hour: u8,
    minute: u8,
    second: u8,
    fraction: Option<(usize, usize)>,
    offset: Option<i16>,
}
impl Admitted {
    fn write(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len().saturating_add(9));
        if self.date_end > 0 {
            out.push_str(&text[..self.date_end]);
        }
        if self.date_end == text.len() {
            return out;
        }
        if self.date_end > 0 {
            out.push('T');
        }
        write!(
            out,
            "{:02}:{:02}:{:02}",
            self.hour, self.minute, self.second
        )
        .expect("String writes are infallible");
        if let Some((start, end)) = self.fraction {
            out.push('.');
            out.push_str(&text[start..end]);
        }
        if let Some(offset) = self.offset {
            if offset == 0 {
                out.push('Z');
            } else {
                let magnitude = offset.unsigned_abs();
                write!(
                    out,
                    "{}{:02}:{:02}",
                    if offset < 0 { '-' } else { '+' },
                    magnitude / 60,
                    magnitude % 60
                )
                .expect("String writes are infallible");
            }
        }
        out
    }
}
fn pair(bytes: &[u8], at: usize) -> Result<u8, TemporalError> {
    let digits = bytes
        .get(at..at + 2)
        .ok_or(TemporalError("expected two ASCII digits"))?;
    if !digits.iter().all(u8::is_ascii_digit) {
        return Err(TemporalError("expected two ASCII digits"));
    }
    Ok((digits[0] - b'0') * 10 + digits[1] - b'0')
}
fn admit(kind: TemporalKind, text: &str) -> Result<Admitted, TemporalError> {
    let b = text.as_bytes();
    let mut p = 0;
    if kind != TemporalKind::Time {
        let negative = b.first() == Some(&b'-');
        p = usize::from(negative);
        let year = u16::from(pair(b, p)?) * 100 + u16::from(pair(b, p + 2)?);
        if negative && year == 0 {
            return Err(TemporalError("negative year zero is not supported"));
        }
        p += 4;
        if b.get(p) != Some(&b'-') || b.get(p + 3) != Some(&b'-') {
            return Err(TemporalError("expected an extended calendar date"));
        }
        let month = pair(b, p + 1)?;
        let day = pair(b, p + 4)?;
        let max = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if year.is_multiple_of(4)
                && (!year.is_multiple_of(100) || year.is_multiple_of(400)) =>
            {
                29
            }
            2 => 28,
            _ => return Err(TemporalError("invalid calendar month")),
        };
        if day == 0 || day > max {
            return Err(TemporalError("invalid Gregorian calendar day"));
        }
        p += 6;
    }
    let date_end = p;
    if kind == TemporalKind::Date {
        if p != b.len() {
            return Err(TemporalError("unexpected content after date"));
        }
        return Ok(Admitted {
            canonical: true,
            date_end,
            hour: 0,
            minute: 0,
            second: 0,
            fraction: None,
            offset: None,
        });
    }
    let mut canonical = true;
    if kind == TemporalKind::DateTime {
        if b.get(p) != Some(&b'T') {
            return Err(TemporalError("datetime requires uppercase T"));
        }
        p += 1;
    } else if b.get(p) == Some(&b'T') {
        p += 1;
        canonical = false;
    }
    let hour = pair(b, p)?;
    p += 2;
    let mut minute = 0;
    let mut second = 0;
    let mut seconds = false;
    if b.get(p) == Some(&b':') {
        minute = pair(b, p + 1)?;
        p += 3;
        if b.get(p) == Some(&b':') {
            second = pair(b, p + 1)?;
            p += 3;
            seconds = true;
        }
    }
    canonical &= seconds;
    if hour > 24 || minute > 59 || second > 60 {
        return Err(TemporalError("clock field out of range"));
    }
    let fraction = if matches!(b.get(p), Some(b'.' | b',')) {
        if !seconds {
            return Err(TemporalError("fractions are supported only on seconds"));
        }
        canonical &= b[p] == b'.';
        p += 1;
        let start = p;
        while b.get(p).is_some_and(u8::is_ascii_digit) {
            p += 1;
        }
        if p == start {
            return Err(TemporalError("fraction requires ASCII digits"));
        }
        Some((start, p))
    } else {
        None
    };
    if hour == 24
        && (minute != 0
            || second != 0
            || fraction.is_some_and(|(start, end)| b[start..end].iter().any(|&v| v != b'0')))
    {
        return Err(TemporalError("end-of-day requires zero lower fields"));
    }
    let offset = match b.get(p) {
        None => None,
        Some(b'Z') => {
            p += 1;
            Some(0)
        }
        Some(sign @ (b'+' | b'-')) => {
            let negative = *sign == b'-';
            let h = pair(b, p + 1)?;
            p += 3;
            let m = if b.get(p) == Some(&b':') {
                let m = pair(b, p + 1)?;
                p += 3;
                m
            } else {
                canonical = false;
                0
            };
            if h > 23 || m > 59 || (negative && h == 0 && m == 0) {
                return Err(TemporalError("invalid ISO offset"));
            }
            let magnitude = i16::from(h) * 60 + i16::from(m);
            canonical &= magnitude != 0;
            Some(if negative { -magnitude } else { magnitude })
        }
        _ => return Err(TemporalError("invalid clock or offset spelling")),
    };
    if p != b.len() {
        return Err(TemporalError("unexpected content after clock"));
    }
    if second == 60
        && offset.is_some_and(|offset| {
            (i16::from(hour) * 60 + i16::from(minute) - offset).rem_euclid(1440) != 1439
        })
    {
        return Err(TemporalError(
            "offset leap-second label must correspond to UTC minute 23:59",
        ));
    }
    Ok(Admitted {
        canonical,
        date_end,
        hour,
        minute,
        second,
        fraction,
        offset,
    })
}
