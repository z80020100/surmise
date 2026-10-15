//! Shell word quoting.
//!
//! The picker hands a line back to the shell. A directory name that holds a
//! space or a quote therefore has to survive the shell's own word splitting.
//! `quote` puts such a name in single quotes and `unquote` takes a person's
//! own quoting back off. `expand` reads the value a line gives a variable the
//! way the shell would.

use std::ffi::OsString;

/// Characters a shell leaves alone in the middle of a word. `=` is not one of
/// them: `MAGIC_EQUAL_SUBST` expands what follows an `=` anywhere in the word
/// and a name like `cache=~old` then reaches for another user's home.
const SAFE: &str = "/._-~+:@,";

/// Quote `s` so the shell reads it as one literal word.
///
/// A word built only from safe characters comes back unchanged. Anything else
/// goes in single quotes. A leading `~` or `-` counts as unsafe even though
/// the character is safe further in: zsh expands the first and `cd` reads the
/// second as a flag. A caller that wants the shell to expand the word must
/// therefore keep it away from here.
///
/// Quoting does not reach `cd +2` or `cd -2`. `cd` reads its own argument
/// after the shell has taken the quotes off and a place in the directory stack
/// is what it sees either way. `candidates::dir_stack_spec` is where that is
/// answered.
pub fn quote(s: &str) -> String {
    let expands = s.starts_with(['~', '=', '-', '+']);
    let plain = |c: char| c.is_alphanumeric() || SAFE.contains(c);
    if !s.is_empty() && !expands && s.chars().all(plain) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Quote `s` for the middle of a word, behind an option's separator. A
/// leading `-` or `+` reads as itself there. Only `~` still expands:
/// `MAGIC_EQUAL_SUBST` reaches past an `=`.
pub fn quote_within(s: &str) -> String {
    let plain = |c: char| c.is_alphanumeric() || SAFE.contains(c);
    if !s.is_empty() && !s.starts_with('~') && s.chars().all(plain) {
        return s.to_string();
    }
    quote(s)
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

/// Whether `s` can name a shell variable.
pub fn is_name(s: &str) -> bool {
    !s.starts_with(|c: char| c.is_ascii_digit()) && !s.is_empty() && s.chars().all(is_name_char)
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// What `value`, written on the right of an assignment, holds once the shell
/// expands it. `var` gives what a variable holds. `None` is a value only
/// running something could tell, such as a `$(…)`. It is also a variable
/// `var` does not hold, because the shell can hold one it does not export.
///
/// A `~` that leads the value or follows a `:` is `$HOME`. zsh reads it so in
/// an assignment and so does bash. `~name` is another user's home and gives
/// `None`. So does every `$` that names no variable.
pub fn expand(value: &str, var: impl Fn(&str) -> Option<OsString>) -> Option<OsString> {
    let mut out = OsString::new();
    let mut chars = value.chars().peekable();
    let mut in_double_quotes = false;
    let mut tilde_may_start = true;
    while let Some(c) = chars.next() {
        let tilde_starts = std::mem::replace(&mut tilde_may_start, false) && !in_double_quotes;
        match c {
            '\'' if !in_double_quotes => {
                for c in chars.by_ref().take_while(|&c| c != '\'') {
                    out.push(c.encode_utf8(&mut [0; 4]));
                }
            }
            '"' => in_double_quotes = !in_double_quotes,
            '\\' => match chars.next() {
                Some(c) if !in_double_quotes || crate::shellparse::escapes_in_double_quotes(c) => {
                    out.push(c.encode_utf8(&mut [0; 4]));
                }
                Some(c) => {
                    out.push("\\");
                    out.push(c.encode_utf8(&mut [0; 4]));
                }
                None => {}
            },
            '`' => return None,
            '$' => {
                let name: String = if chars.next_if_eq(&'{').is_some() {
                    let mut name = String::new();
                    loop {
                        match chars.next()? {
                            '}' => break name,
                            c => name.push(c),
                        }
                    }
                } else {
                    std::iter::from_fn(|| chars.next_if(|&c| is_name_char(c))).collect()
                };
                if !is_name(&name) {
                    return None;
                }
                out.push(var(&name)?);
            }
            '~' if tilde_starts => match chars.peek() {
                None | Some('/' | ':') => out.push(var("HOME")?),
                Some(_) => return None,
            },
            ':' if !in_double_quotes => {
                out.push(":");
                tilde_may_start = true;
            }
            c => out.push(c.encode_utf8(&mut [0; 4])),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::{expand, quote, quote_within, unquote};
    use std::ffi::OsString;

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
    fn a_value_inside_a_word_keeps_a_leading_sign() {
        assert_eq!(quote_within("+x"), "+x");
        assert_eq!(quote_within("-x"), "-x");
        assert_eq!(quote_within("~old"), "'~old'");
        assert_eq!(quote_within("a b"), "'a b'");
    }

    #[test]
    fn an_empty_word_still_has_to_be_a_word() {
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn a_character_the_shell_would_expand_is_only_unsafe_in_front() {
        assert_eq!(quote("~"), "'~'");
        assert_eq!(quote("-x/"), "'-x/'");
        assert_eq!(quote("+x/"), "'+x/'");
        // `cd +2` is the second entry of the directory stack rather than a
        // directory of that name. The quotes do not settle that and
        // `candidates::dir_stack_spec` is what does.
        assert_eq!(quote("+2/"), "'+2/'");
        assert_eq!(quote("a~b/"), "a~b/");
        assert_eq!(quote("a-b/"), "a-b/");
        assert_eq!(quote("a+b/"), "a+b/");
    }

    #[test]
    fn an_equals_sign_is_unsafe_wherever_it_sits() {
        // `MAGIC_EQUAL_SUBST` is off by default and a `.zshrc` that sets it is
        // ordinary. It expands what follows an `=` anywhere in the word and
        // `cache=~old` then reaches for a user named `old`.
        assert_eq!(quote("=x/"), "'=x/'");
        assert_eq!(quote("a=b/"), "'a=b/'");
        assert_eq!(quote("cache=~old/"), "'cache=~old/'");
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
            "a=b",
            "-x",
            "+2",
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

    /// What `value` expands to with `HOME` at `/home/sample` and `SAMPLE_DIR`
    /// at `/sample`. `None` is a value only running something could tell.
    fn expanded(value: &str) -> Option<String> {
        let var = |name: &str| match name {
            "HOME" => Some(OsString::from("/home/sample")),
            "SAMPLE_DIR" => Some(OsString::from("/sample")),
            _ => None,
        };
        expand(value, var).map(|value| value.into_string().unwrap())
    }

    #[test]
    fn a_plain_value_stays_as_written() {
        assert_eq!(expanded("/sample/bin").as_deref(), Some("/sample/bin"));
        assert_eq!(expanded("").as_deref(), Some(""));
        assert_eq!(expanded("a*b").as_deref(), Some("a*b"));
    }

    #[test]
    fn quotes_and_escapes_come_off() {
        assert_eq!(expanded("'/sample dir'").as_deref(), Some("/sample dir"));
        assert_eq!(expanded("\"/sample dir\"").as_deref(), Some("/sample dir"));
        assert_eq!(expanded(r"/sample\ dir").as_deref(), Some("/sample dir"));
        assert_eq!(expanded(r#""a\b\$c""#).as_deref(), Some(r"a\b$c"));
    }

    #[test]
    fn a_variable_reads_what_it_holds() {
        assert_eq!(expanded("$SAMPLE_DIR/bin").as_deref(), Some("/sample/bin"));
        assert_eq!(expanded("${SAMPLE_DIR}bin").as_deref(), Some("/samplebin"));
        assert_eq!(
            expanded("\"$HOME/a b\"").as_deref(),
            Some("/home/sample/a b")
        );
        assert_eq!(expanded("'$HOME'").as_deref(), Some("$HOME"));
        assert_eq!(expanded(r"\$HOME").as_deref(), Some("$HOME"));
    }

    #[test]
    fn a_tilde_reads_home_where_the_shell_expands_one() {
        assert_eq!(expanded("~").as_deref(), Some("/home/sample"));
        assert_eq!(
            expanded("~/a:~/b").as_deref(),
            Some("/home/sample/a:/home/sample/b")
        );
        assert_eq!(expanded("a:~").as_deref(), Some("a:/home/sample"));
        assert_eq!(expanded("x~/y").as_deref(), Some("x~/y"));
        assert_eq!(expanded("\"~/a\"").as_deref(), Some("~/a"));
        assert_eq!(expanded("'~'/a").as_deref(), Some("~/a"));
    }

    #[test]
    fn what_only_running_something_could_tell_gives_none() {
        for value in [
            "$(sample)",
            "`sample`",
            "$SAMPLE_UNSET",
            "${SAMPLE_DIR:-/x}",
            "${SAMPLE_DIR",
            "$1",
            "$$",
            "$'a'",
            "a$",
            "~sample",
            "~+",
        ] {
            assert_eq!(expanded(value), None, "{value:?}");
        }
    }
}
