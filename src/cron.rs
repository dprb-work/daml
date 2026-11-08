//! Spring Framework 7.0.10 CronExpression admission, without a scheduler.
//!
//! Independently authored from the public parser contract and observed behavior;
//! see SOURCES.md for reference provenance. Retained text is deliberately not a
//! normalized set of occurrences: both day fields constrain the same occurrence.
use std::borrow::Cow;
use unicode_general_category::{GeneralCategory, get_general_category};

const MONTHS: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const DAYS: [&str; 7] = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];
const ORDINALS: [&str; 12] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12",
];

// String.trim(), not Unicode whitespace trimming. Spring also trims each token
// after splitting on literal ASCII spaces; tabs alone are not field separators.
fn trim(text: &str) -> &str {
    text.trim_matches(|ch: char| ch <= '\u{20}')
}

fn macro_name(value: &str, name: &str) -> bool {
    // Java equalsIgnoreCase uses simple character mappings, including dotted
    // and dotless I, rather than ASCII-only case folding.
    if value.is_ascii() {
        return value.eq_ignore_ascii_case(name);
    }
    value.encode_utf16().count() == name.len()
        && value.chars().zip(name.chars()).all(|(ch, expected)| {
            ch == expected
                || ch
                    .to_uppercase()
                    .eq(std::iter::once(expected.to_ascii_uppercase()))
                || ch.to_lowercase().eq(std::iter::once(expected))
                || (ch == '\u{130}' && expected == 'i')
        })
}

pub(crate) fn admit(text: Cow<'_, str>) -> Result<Cow<'_, str>, &'static str> {
    let retained = trim(&text);
    let expansion = if !retained.starts_with('@') {
        None
    } else if macro_name(retained, "@yearly") || macro_name(retained, "@annually") {
        Some("0 0 0 1 1 *")
    } else if macro_name(retained, "@monthly") {
        Some("0 0 0 1 * *")
    } else if macro_name(retained, "@weekly") {
        Some("0 0 0 * * 0")
    } else if macro_name(retained, "@daily") || macro_name(retained, "@midnight") {
        Some("0 0 0 * * *")
    } else if macro_name(retained, "@hourly") {
        Some("0 0 * * * *")
    } else {
        None
    };
    if let Some(expansion) = expansion {
        return Ok(Cow::Borrowed(expansion));
    }
    let mut fields = retained
        .split(' ')
        .map(trim)
        .filter(|part| !part.is_empty());
    for index in 0..6 {
        field(fields.next().ok_or("cron requires six fields")?, index)?;
    }
    if fields.next().is_some() {
        return Err("cron requires six fields");
    }
    if retained.len() == text.len() {
        return Ok(text);
    }
    let start = retained.as_ptr() as usize - text.as_ptr() as usize;
    let end = start + retained.len();
    Ok(match text {
        Cow::Borrowed(text) => Cow::Borrowed(&text[start..end]),
        Cow::Owned(mut text) => {
            text.truncate(end);
            text.drain(..start);
            Cow::Owned(text)
        }
    })
}

// Spring uppercases month/week fields and replaces every ordinal substring,
// even in concatenated numeric spellings. This temporary form is for admission
// only; the retained expression keeps its original spelling and field layout.
fn ordinals<'a>(value: &'a str, names: &[&str]) -> Cow<'a, str> {
    let mut value = if value
        .chars()
        .any(|ch| !ch.to_uppercase().eq(std::iter::once(ch)))
    {
        Cow::Owned(value.to_uppercase())
    } else {
        Cow::Borrowed(value)
    };
    for (name, ordinal) in names.iter().copied().zip(ORDINALS) {
        if value.contains(name) {
            value = Cow::Owned(value.replace(name, ordinal));
        }
    }
    value
}

fn field(value: &str, index: usize) -> Result<(), &'static str> {
    let value = match index {
        4 => ordinals(value, &MONTHS),
        5 => ordinals(value, &DAYS),
        _ => Cow::Borrowed(value),
    };
    let calendar = |part: &str| match index {
        3 => part.contains(['L', 'W']),
        5 => part.contains(['L', '#']),
        _ => false,
    };
    if calendar(&value) {
        for part in value.split(',') {
            if calendar(part) {
                match index {
                    3 => month_day(part)?,
                    5 => week_day(part)?,
                    _ => unreachable!("only day fields have calendar forms"),
                }
            } else {
                ordinary(part, index)?;
            }
        }
        Ok(())
    } else {
        ordinary(&value, index)
    }
}

fn month_day(value: &str) -> Result<(), &'static str> {
    if let Some(last) = value.rfind('L') {
        if last != 0 {
            return Err("invalid characters before cron L");
        }
        if value == "L" || value == "LW" || integer(&value[1..])? < 0 {
            return Ok(());
        }
        return Err("cron L offset must be negative");
    }
    if let Some(day) = value.strip_suffix('W')
        && (1..=31).contains(&integer(day)?)
    {
        return Ok(());
    }
    Err("invalid cron nearest weekday")
}

fn week_day(value: &str) -> Result<(), &'static str> {
    let day = if let Some(last) = value.rfind('L') {
        if last != value.len() - 1 {
            return Err("invalid characters after cron L");
        }
        &value[..last]
    } else if let Some((day, ordinal)) = value.rsplit_once('#') {
        if integer(ordinal)? <= 0 {
            return Err("cron weekday ordinal must be positive");
        }
        day
    } else {
        return Err("invalid cron calendar weekday");
    };
    if (0..=7).contains(&integer(day)?) {
        Ok(())
    } else {
        Err("cron weekday is out of range")
    }
}

fn ordinary(value: &str, index: usize) -> Result<(), &'static str> {
    // '?' is a whole day-field alias, not an ordinary list/range operand.
    if matches!(index, 3 | 5) && value == "?" {
        return Ok(());
    }
    for part in value.split(',') {
        let range = if let Some((range, step)) = part.split_once('/') {
            if integer(step)? <= 0 {
                return Err("cron increment must be positive");
            }
            range
        } else {
            part
        };
        if range == "*" {
            continue;
        }
        let (min, max) = match index {
            0 | 1 => (0, 59),
            2 => (0, 23),
            3 => (1, 31),
            4 => (1, 12),
            5 => (0, 7),
            _ => unreachable!("cron has exactly six fields"),
        };
        let checked = |text| {
            let value = integer(text)?;
            if (min..=max).contains(&value) {
                Ok(value)
            } else {
                Err("cron field value is out of range")
            }
        };
        if let Some((start, end)) = range.split_once('-') {
            let mut start = checked(start)?;
            let end = checked(end)?;
            if index == 5 && start == 7 {
                start = 0;
            }
            if start > end {
                return Err("cron range is reversed");
            }
        } else {
            checked(range)?;
        }
    }
    Ok(())
}

// Integer.parseInt admits ASCII signs and BMP Unicode decimal digits, with
// signed 32-bit overflow checks. Supplementary digits occupy surrogate pairs in
// Java and are not admitted by that parser. No locale or numeric coercion is used.
fn integer(value: &str) -> Result<i32, &'static str> {
    let (negative, digits) = if let Some(value) = value.strip_prefix('-') {
        (true, value)
    } else {
        (false, value.strip_prefix('+').unwrap_or(value))
    };
    if digits.is_empty() {
        return Err("cron integer requires digits");
    }
    let mut number = 0i32;
    for ch in digits.chars() {
        let digit = if ch.is_ascii_digit() {
            i32::from(ch as u8 - b'0')
        } else if ch <= '\u{ffff}' && get_general_category(ch) == GeneralCategory::DecimalNumber {
            let mut zero = ch as u32;
            while char::from_u32(zero - 1)
                .is_some_and(|ch| get_general_category(ch) == GeneralCategory::DecimalNumber)
            {
                zero -= 1;
            }
            (ch as u32 - zero) as i32
        } else {
            return Err("invalid cron integer");
        };
        number = number
            .checked_mul(10)
            .and_then(|number| number.checked_sub(digit))
            .ok_or("cron integer exceeds signed 32-bit range")?;
    }
    if negative {
        Ok(number)
    } else {
        number
            .checked_neg()
            .ok_or("cron integer exceeds signed 32-bit range")
    }
}
