//! Text and domain rules shared by the index and the block list (security-2 S12, S14).

/// Unicode `General_Category=Cf` (format characters: bidi overrides and isolates, zero-width
/// characters, tags, the BOM), as of Unicode 15.1. They change how text around them is shown,
/// so a name or reason could hide or reorder words.
const FORMAT_CHARACTERS: &[(u32, u32)] = &[
    (0x00AD, 0x00AD),
    (0x0600, 0x0605),
    (0x061C, 0x061C),
    (0x06DD, 0x06DD),
    (0x070F, 0x070F),
    (0x0890, 0x0891),
    (0x08E2, 0x08E2),
    (0x180E, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x2066, 0x206F),
    (0xFEFF, 0xFEFF),
    (0xFFF9, 0xFFFB),
    (0x110BD, 0x110BD),
    (0x110CD, 0x110CD),
    (0x13430, 0x1343F),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0001, 0xE0001),
    (0xE0020, 0xE007F),
];

/// Characters that never belong in text shown to the user: controls (Cc), format characters
/// (Cf), and the line and paragraph separators U+2028 and U+2029.
pub(crate) fn is_hidden_character(character: char) -> bool {
    let code = u32::from(character);
    character.is_control()
        || code == 0x2028
        || code == 0x2029
        || FORMAT_CHARACTERS
            .iter()
            .any(|&(first, last)| (first..=last).contains(&code))
}

/// Free text (descriptions, block reasons): `min` to `max` characters, no hidden characters, no
/// leading or trailing whitespace.
pub(crate) fn check_text(field: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    let length = value.chars().count();
    if length < min || length > max {
        return Err(format!("{field} must be {min} to {max} characters"));
    }
    if value.chars().any(is_hidden_character) {
        return Err(format!(
            "{field} must not contain control, format (bidi, zero-width) or separator characters"
        ));
    }
    if value.trim() != value {
        return Err(format!("{field} must not start or end with whitespace"));
    }
    Ok(())
}

/// A display name, which the extensions page shows next to the publisher: [`check_text`], and
/// only letters, digits, spaces and plain punctuation, so it cannot carry symbols that look like
/// host badges. Confusable letters of other scripts remain possible; the publisher login is shown
/// next to it.
pub(crate) fn check_name(field: &str, value: &str, max: usize) -> Result<(), String> {
    check_text(field, value, 1, max)?;
    let allowed = |character: char| {
        character.is_alphanumeric() || character == ' ' || "-_.,:;!?&+'()/#".contains(character)
    };
    if let Some(character) = value.chars().find(|&character| !allowed(character)) {
        return Err(format!(
            "{field} may only use letters, digits, spaces and plain punctuation, not {character:?}"
        ));
    }
    if value.contains("  ") {
        return Err(format!("{field} must not contain repeated spaces"));
    }
    Ok(())
}

/// The public suffix rule now lives in `ui_ext`, which the host's `network:fetch` domain list
/// shares with the registry.
pub use ui_ext::network::is_public_suffix;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_characters() {
        for hidden in [
            '\u{1b}',
            '\u{202e}',
            '\u{2066}',
            '\u{200b}',
            '\u{feff}',
            '\u{2028}',
            '\u{e0041}',
        ] {
            assert!(is_hidden_character(hidden), "{hidden:?}");
        }
        for shown in ['a', 'é', '中', ' ', '-', '\u{2014}'] {
            assert!(!is_hidden_character(shown), "{shown:?}");
        }
    }

    #[test]
    fn names() {
        for good in [
            "Todo",
            "Git Blame (beta)",
            "Écrire",
            "中文助手",
            "C++ tools",
            "Notes #2",
        ] {
            assert_eq!(check_name("name", good, 64), Ok(()), "{good}");
        }
        for bad in [
            "",
            " Todo",
            "Todo\u{202e}",
            "Todo \u{2714}",
            "Verified\u{2705}",
            "a  b",
        ] {
            assert!(check_name("name", bad, 64).is_err(), "{bad:?}");
        }
    }
}
