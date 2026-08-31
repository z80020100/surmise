//! Shell word quoting.
//!
//! The picker hands a line back to the shell. A directory name that holds a
//! space or a quote therefore has to survive the shell's own word splitting.
//! `quote` puts such a name in single quotes and `unquote` takes a person's
//! own quoting back off.

/// Characters a shell leaves alone in the middle of a word.
const SAFE: &str = "/._-~+=:@,";

/// Quote `s` so the shell reads it as one literal word.
///
/// A word built only from safe characters comes back unchanged. Anything else
/// goes in single quotes. A leading `~`, `=` or `-` counts as unsafe even
/// though the character is safe further in: zsh expands the first two and `cd`
/// reads the third as a flag. A caller that wants the shell to expand the word
/// must therefore keep it away from here.
pub fn quote(s: &str) -> String {
    let expands = matches!(s.chars().next(), Some('~' | '=' | '-'));
    let plain = |c: char| c.is_alphanumeric() || SAFE.contains(c);
    if !s.is_empty() && !expands && s.chars().all(plain) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Strip the shell quoting a person typed, including a quote they have not
/// closed yet.
pub fn unquote(s: &str) -> String {
    let t = s.trim();
    let Some(q @ ('\'' | '"')) = t.chars().next() else {
        return t.to_string();
    };
    let body = &t[q.len_utf8()..];
    let body = body.strip_suffix(q).unwrap_or(body);
    // `'\''` is how a single-quoted word carries a single quote. Inside a
    // double-quoted word the same four characters are literal.
    if q == '"' {
        return body.to_string();
    }
    body.replace(r"'\''", "'")
}

#[cfg(test)]
mod tests {
    use super::{quote, unquote};

    #[test]
    fn a_plain_name_needs_no_quotes() {
        assert_eq!(quote("work"), "work");
        assert_eq!(quote("work/alpha/"), "work/alpha/");
        assert_eq!(quote("a-b_c.d/"), "a-b_c.d/");
        assert_eq!(quote("好work/"), "好work/");
    }

    #[test]
    fn a_space_forces_quotes() {
        assert_eq!(quote("my docs/"), "'my docs/'");
        assert_eq!(quote("a\tb"), "'a\tb'");
    }

    #[test]
    fn an_embedded_single_quote_is_escaped() {
        assert_eq!(quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn an_empty_word_still_has_to_be_a_word() {
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn a_character_the_shell_would_expand_is_only_unsafe_in_front() {
        assert_eq!(quote("~"), "'~'");
        assert_eq!(quote("=x/"), "'=x/'");
        assert_eq!(quote("-x/"), "'-x/'");
        assert_eq!(quote("a=b/"), "a=b/");
        assert_eq!(quote("a~b/"), "a~b/");
        assert_eq!(quote("a-b/"), "a-b/");
    }

    #[test]
    fn a_glob_character_forces_quotes() {
        assert_eq!(quote("a*b"), "'a*b'");
        assert_eq!(quote("a?b"), "'a?b'");
        assert_eq!(quote("a[b]"), "'a[b]'");
        assert_eq!(quote("a$b"), "'a$b'");
    }

    #[test]
    fn unquote_takes_single_quotes_off() {
        assert_eq!(unquote("'my docs'"), "my docs");
        assert_eq!(unquote(r"'it'\''s'"), "it's");
    }

    #[test]
    fn unquote_takes_double_quotes_off() {
        assert_eq!(unquote("\"my docs\""), "my docs");
    }

    #[test]
    fn unquote_leaves_an_escape_alone_inside_double_quotes() {
        assert_eq!(unquote(r#""a'\''b""#), r"a'\''b");
    }

    #[test]
    fn unquote_handles_a_quote_that_is_still_open() {
        assert_eq!(unquote("'my do"), "my do");
        assert_eq!(unquote("\"my do"), "my do");
        assert_eq!(unquote("'"), "");
    }

    #[test]
    fn unquote_leaves_a_bare_word_alone() {
        assert_eq!(unquote("work"), "work");
        assert_eq!(unquote("  work  "), "work");
        assert_eq!(unquote(""), "");
    }

    #[test]
    fn unquote_keeps_the_space_that_the_quotes_protected() {
        assert_eq!(unquote("  'my docs'  "), "my docs");
    }

    #[test]
    fn every_awkward_name_survives_a_round_trip() {
        for name in [
            "work",
            "my docs",
            "it's",
            "",
            "~",
            "=x",
            "-x",
            "a b'c\"d",
            "好 work",
            "a*b?c[d]",
            " leading",
            "trailing ",
            "a\nb",
            "'leading quote",
            "back\\slash",
        ] {
            assert_eq!(unquote(&quote(name)), name, "round trip failed: {name:?}");
        }
    }
}
