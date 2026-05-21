/*!
 Defines routines for sanitizing text data.
*/

use std::borrow::Cow;

use askama::filters::Escaper;

use crate::app::escaping::ChatEscaper;

/// The character to replace disallowed chars with
const FILENAME_REPLACEMENT_CHAR: char = '_';

/// Returns true if a character is disallowed in filenames
#[inline]
fn is_filename_disallowed(c: char) -> bool {
    matches!(c, '*' | '"' | '/' | '\\' | '<' | '>' | ':' | '|' | '?')
}

/// Remove unsafe chars in filenames.
///
/// Does not need to use a `Cow` for optimization because the source is always generated based on chat data
/// so there is no opportunity for the original input to be passed in from another borrow.
pub fn sanitize_filename(filename: &str) -> String {
    filename
        .chars()
        .map(|letter| {
            if letter.is_control() || is_filename_disallowed(letter) {
                FILENAME_REPLACEMENT_CHAR
            } else {
                letter
            }
        })
        .collect()
}

/// Escapes HTML special characters in the input string, allocating only if
/// at least one character needs escaping. Wraps [`ChatEscaper`] so the
/// character set and replacements stay aligned with Askama-rendered output.
pub fn sanitize_html(input: &'_ str) -> Cow<'_, str> {
    // Fast scan: every escape target is either a single ASCII byte or the
    // two-byte NBSP (`0xC2 0xA0`). None of these match a UTF-8 continuation
    // byte, so scanning bytes (rather than chars) is safe.
    let bytes = input.as_bytes();
    let needs_escape = bytes.iter().enumerate().any(|(i, &b)| {
        matches!(b, b'<' | b'>' | b'"' | b'\'' | b'`' | b'&')
            || (b == 0xC2 && bytes.get(i + 1) == Some(&0xA0))
    });
    if !needs_escape {
        return Cow::Borrowed(input);
    }
    let mut out = String::with_capacity(input.len() + 8);
    ChatEscaper
        .write_escaped_str(&mut out, input)
        .unwrap_or_default();
    Cow::Owned(out)
}

#[cfg(test)]
mod filename_sanitization_tests {
    use crate::app::sanitizers::sanitize_filename;

    #[test]
    fn can_sanitize_macos() {
        assert_eq!(sanitize_filename("a/b\\c:d"), "a_b_c_d");
    }

    #[test]
    fn doesnt_sanitize_none() {
        assert_eq!(sanitize_filename("a_b_c_d"), "a_b_c_d");
    }

    #[test]
    fn can_sanitize_one() {
        assert_eq!(sanitize_filename("ab/cd"), "ab_cd");
    }

    #[test]
    fn can_sanitize_only_bad() {
        assert_eq!(
            sanitize_filename("* \" / \\ < > : | ?"),
            "_ _ _ _ _ _ _ _ _"
        );
    }

    #[test]
    fn handles_emoji() {
        assert_eq!(sanitize_filename("hello🌍world"), "hello🌍world");
    }

    #[test]
    fn handles_cyrillic() {
        assert_eq!(sanitize_filename("привет/мир"), "привет_мир");
    }

    #[test]
    fn handles_leading_space() {
        assert_eq!(sanitize_filename(" leading space"), " leading space");
    }

    #[test]
    fn handles_trailing_space() {
        assert_eq!(sanitize_filename("trailing space "), "trailing space ");
    }

    #[test]
    fn handles_tab_char() {
        assert_eq!(sanitize_filename("tab\there"), "tab_here");
    }

    #[test]
    fn handles_newline() {
        assert_eq!(sanitize_filename("new\nline"), "new_line");
    }

    #[test]
    fn handles_carriage_return() {
        assert_eq!(sanitize_filename("return\r"), "return_");
    }

    #[test]
    fn handles_ascii_controls() {
        assert_eq!(sanitize_filename("ascii\x01\x1F"), "ascii__");
    }

    #[test]
    fn handles_empty_string() {
        assert_eq!(sanitize_filename(""), "");
    }

    #[test]
    fn leaves_allowed_chars_unchanged() {
        assert_eq!(sanitize_filename("file.name-version"), "file.name-version");
    }

    #[test]
    fn handles_accented_letters() {
        assert_eq!(sanitize_filename("café/niño"), "café_niño");
    }

    #[test]
    fn replaces_del_control_char() {
        assert_eq!(sanitize_filename("\x7F"), "_");
    }

    #[test]
    fn handles_mixed_control_and_disallowed() {
        assert_eq!(sanitize_filename("*\t?\r"), "____");
    }

    #[test]
    fn handles_chinese() {
        assert_eq!(sanitize_filename("你好/世界"), "你好_世界");
    }
}

#[cfg(test)]
mod html_sanitization_tests {
    use std::borrow::Cow;

    use crate::app::sanitizers::sanitize_html;

    // Character-set / replacement behavior is covered by
    // `app::escaping::tests`. These tests cover only what `sanitize_html`
    // itself adds: the `Cow` fast path and the integration with
    // `ChatEscaper`.

    #[test]
    fn empty_input_borrows() {
        assert!(matches!(sanitize_html(""), Cow::Borrowed(_)));
    }

    #[test]
    fn no_escape_targets_borrows() {
        let input = "Hello world, привет, 🌍";
        match sanitize_html(input) {
            Cow::Borrowed(s) => assert_eq!(s.as_ptr(), input.as_ptr()),
            Cow::Owned(_) => panic!("expected Borrowed for input with no escape targets"),
        }
    }

    #[test]
    fn ascii_escape_target_allocates() {
        assert!(matches!(sanitize_html("<"), Cow::Owned(_)));
    }

    #[test]
    fn nbsp_escape_target_allocates() {
        assert!(matches!(sanitize_html("\u{a0}"), Cow::Owned(_)));
    }

    #[test]
    fn matches_chat_escaper_on_mixed_content() {
        // One end-to-end smoke test through the wrapper. The full character
        // set is covered by `ChatEscaper`'s tests.
        assert_eq!(
            &sanitize_html("<div>Hello\u{a0}&amp;\u{a0}world</div>"),
            "&lt;div&gt;Hello&nbsp;&amp;amp;&nbsp;world&lt;/div&gt;"
        );
    }
}
