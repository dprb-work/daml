use unicode_general_category::{GeneralCategory, get_general_category};

pub(crate) fn valid_key(value: &str) -> bool {
    valid_name(value, true)
}

pub(crate) fn valid_segment(value: &str) -> bool {
    valid_name(value, false)
}

pub(crate) fn valid_tag(value: &str) -> bool {
    let mut segments = value.split('.');
    segments.next().is_some_and(valid_segment) && segments.all(valid_segment)
}

fn valid_name(value: &str, key: bool) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    name_start(first, key) && chars.all(|ch| name_continue(ch, key))
}

fn name_start(ch: char, key: bool) -> bool {
    name_character(ch, key, false)
}

fn name_continue(ch: char, key: bool) -> bool {
    name_character(ch, key, true)
}

fn name_character(ch: char, key: bool, allow_mark: bool) -> bool {
    if ch.is_ascii() {
        return ch.is_ascii_alphanumeric()
            || matches!(ch, '_' | '-')
            || (key && matches!(ch, '.' | '/'));
    }
    match get_general_category(ch) {
        GeneralCategory::UppercaseLetter
        | GeneralCategory::LowercaseLetter
        | GeneralCategory::TitlecaseLetter
        | GeneralCategory::ModifierLetter
        | GeneralCategory::OtherLetter
        | GeneralCategory::DecimalNumber
        | GeneralCategory::LetterNumber
        | GeneralCategory::OtherNumber => true,
        GeneralCategory::NonspacingMark
        | GeneralCategory::SpacingMark
        | GeneralCategory::EnclosingMark => allow_mark,
        _ => false,
    }
}
