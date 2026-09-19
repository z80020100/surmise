//! Splitting a shell line into commands and words.
//!
//! A menu completes the word the cursor sits in. Finding that word means
//! reading the whole line the way the shell would: quotes swallow whitespace
//! and operators, and a leading `FOO=bar` is not the command's name. This
//! module does that reading and stops there. It does not run anything and it
//! does not know what a command's arguments mean.
//!
//! The line is read once into a flat sequence of [`Command`]s, each carrying
//! the [`Operator`] that follows it. That is enough to walk one command's
//! words, which is the whole job of this slice. A pipeline binds tighter than
//! a list, and a later pass can still see that: it groups a run of commands
//! joined by [`Operator::Pipe`] or [`Operator::PipeAmp`] into one pipeline,
//! then groups a run of pipelines joined by anything else into one list. This
//! module also stays clear of `$(…)`, `` `…` ``, `$((…))` and `${…}`, which
//! sit as plain text inside whichever token they fall in. An unquoted space or
//! operator inside one of those is read as ending the token or the command
//! the same as anywhere else. Giving them their own structure, so that a
//! space inside `$(…)` stays inside it, is `Subshell`, `CompoundStatement`,
//! `ArithmeticExpansion`, `CommandSubstitution` and the rest of the structured
//! expansion family. None of them are built here: they are parity work with
//! another shell reader rather than completion work, and a menu loses nothing
//! today from reading `$(a; b)` as two commands instead of one.

/// One quoted or bare word: where it sits in the original buffer and what it
/// reads as once its quoting is gone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    /// Byte offset of the first character of the word, quote included.
    pub start: usize,
    /// Byte offset just past the word's last character, quote included.
    pub end: usize,
    /// The word with its quoting and escaping removed.
    pub inner_text: String,
}

/// What ends one command and starts the next. [`Operator::Pipe`] and
/// [`Operator::PipeAmp`] join commands into a pipeline; the rest join
/// pipelines into a list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Semicolon,
    Amp,
    AmpSemicolon,
    Pipe,
    PipeAmp,
    AndAnd,
    OrOr,
}

/// One command: the assignments in front of its name, its name and arguments,
/// and the operator that follows it. `words` is empty for a line that is
/// nothing but assignments so far; otherwise `words[0]` is the command name
/// and the rest are its arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    /// Byte offset where this command starts, including any leading
    /// whitespace after the previous operator.
    pub start: usize,
    /// Byte offset where this command ends, up to but not including the
    /// operator that follows it.
    pub end: usize,
    pub assignments: Vec<Token>,
    pub words: Vec<Token>,
    /// `None` for the last command in the buffer.
    pub terminator: Option<Operator>,
}

/// A line read into its commands, and whether every quote in it closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Parse {
    pub commands: Vec<Command>,
    /// `false` when a quote was still open at the end of the buffer. That is
    /// the ordinary shape of a word someone is still typing rather than an
    /// error, so parsing still returns every command it found.
    pub complete: bool,
}

/// A cursor into `buffer`, advanced one character at a time and always left
/// on a character boundary.
struct Scanner<'a> {
    buf: &'a str,
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(buf: &'a str) -> Self {
        Self { buf, pos: 0 }
    }

    fn peek(&self) -> Option<char> {
        self.buf[self.pos..].chars().next()
    }

    /// The character after the one `peek` would return.
    fn peek_next(&self) -> Option<char> {
        let c = self.peek()?;
        self.buf[self.pos + c.len_utf8()..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }
}

/// A backslash escape inside `"…"`. Anywhere else in a double-quoted word the
/// backslash is a literal character of its own.
fn escapes_in_double_quotes(c: char) -> bool {
    matches!(c, '$' | '`' | '"' | '\\' | '\n')
}

/// The shell's own word separator rather than Unicode's idea of whitespace.
/// `IFS` defaults to space, tab and newline, and nothing else splits an
/// unquoted word: a non-breaking space is an ordinary character of one to
/// the shell, so `char::is_whitespace` would wrongly cut `a<U+00A0>b` in two.
fn is_ifs_whitespace(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n')
}

/// Read a word, continuing one already begun by an earlier quoted or bare
/// piece. `"a"'b'c` is one word built from three such pieces.
fn take_word(word: &mut Option<(usize, String)>, sc: &Scanner) -> (usize, String) {
    word.take().unwrap_or_else(|| (sc.pos, String::new()))
}

fn read_single_quoted(sc: &mut Scanner, text: &mut String, complete: &mut bool) {
    sc.bump(); // the opening quote
    loop {
        match sc.peek() {
            None => {
                *complete = false;
                break;
            }
            Some('\'') => {
                sc.bump();
                break;
            }
            Some(c) => {
                text.push(c);
                sc.bump();
            }
        }
    }
}

fn read_double_quoted(sc: &mut Scanner, text: &mut String, complete: &mut bool) {
    sc.bump(); // the opening quote
    loop {
        match sc.peek() {
            None => {
                *complete = false;
                break;
            }
            Some('"') => {
                sc.bump();
                break;
            }
            Some('\\') if sc.peek_next().is_some_and(escapes_in_double_quotes) => {
                sc.bump();
                text.push(sc.bump().expect("peek_next just confirmed a character"));
            }
            Some(c) => {
                text.push(c);
                sc.bump();
            }
        }
    }
}

/// `$'…'` as a leaf. Its escapes are not decoded in this slice: a `\'`
/// inside it is skipped whole so it cannot close the string early, and
/// whatever the pair reads as goes into `inner_text` unchanged.
fn read_dollar_quoted(sc: &mut Scanner, text: &mut String, complete: &mut bool) {
    sc.bump(); // $
    sc.bump(); // the opening quote
    loop {
        match sc.peek() {
            None => {
                *complete = false;
                break;
            }
            Some('\'') => {
                sc.bump();
                break;
            }
            Some('\\') => {
                text.push(sc.bump().expect("peek confirmed a character"));
                if let Some(c) = sc.bump() {
                    text.push(c);
                }
            }
            Some(c) => {
                text.push(c);
                sc.bump();
            }
        }
    }
}

/// A hand-written stand-in for the gate regex `^[\w[\]]+\+?=.*`, kept by hand
/// because this project carries no regex dependency. JavaScript's `\w` is
/// ASCII-only, so this checks ASCII alphanumerics and `_` rather than every
/// Unicode letter.
fn looks_like_assignment(text: &str) -> bool {
    let is_name_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '[' || c == ']';
    let name_len: usize = text
        .chars()
        .take_while(|&c| is_name_char(c))
        .map(char::len_utf8)
        .sum();
    if name_len == 0 {
        return false;
    }
    let rest = &text[name_len..];
    rest.strip_prefix('+').unwrap_or(rest).starts_with('=')
}

/// Move the leading run of assignment-shaped tokens off the front of
/// `tokens` and into their own list.
fn split_assignments(mut tokens: Vec<Token>) -> (Vec<Token>, Vec<Token>) {
    let split_at = tokens
        .iter()
        .position(|t| !looks_like_assignment(&t.inner_text))
        .unwrap_or(tokens.len());
    let words = tokens.split_off(split_at);
    (tokens, words)
}

/// Close out one command's token list into a [`Command`].
///
/// Whitespace between the last token and `end` with nothing typed after it is
/// the word about to be typed next, and a menu completes exactly that: an
/// empty token positioned at `end`.
fn finish_command(
    buffer: &str,
    start: usize,
    end: usize,
    mut tokens: Vec<Token>,
    terminator: Option<Operator>,
) -> Command {
    let after_last = tokens.last().map_or(start, |t| t.end);
    if end > after_last && buffer[after_last..end].chars().all(is_ifs_whitespace) {
        tokens.push(Token {
            start: end,
            end,
            inner_text: String::new(),
        });
    }
    let (assignments, words) = split_assignments(tokens);
    Command {
        start,
        end,
        assignments,
        words,
        terminator,
    }
}

/// Read `buffer` into its commands.
pub fn parse(buffer: &str) -> Parse {
    let mut sc = Scanner::new(buffer);
    let mut commands = Vec::new();
    let mut complete = true;
    let mut cmd_start = 0;
    let mut tokens: Vec<Token> = Vec::new();
    let mut word: Option<(usize, String)> = None;

    while let Some(c) = sc.peek() {
        match c {
            c if is_ifs_whitespace(c) => {
                if let Some((start, text)) = word.take() {
                    tokens.push(Token {
                        start,
                        end: sc.pos,
                        inner_text: text,
                    });
                }
                sc.bump();
            }
            '\'' => {
                let (start, mut text) = take_word(&mut word, &sc);
                read_single_quoted(&mut sc, &mut text, &mut complete);
                word = Some((start, text));
            }
            '"' => {
                let (start, mut text) = take_word(&mut word, &sc);
                read_double_quoted(&mut sc, &mut text, &mut complete);
                word = Some((start, text));
            }
            '$' if sc.peek_next() == Some('\'') => {
                let (start, mut text) = take_word(&mut word, &sc);
                read_dollar_quoted(&mut sc, &mut text, &mut complete);
                word = Some((start, text));
            }
            '\\' => {
                let (start, mut text) = take_word(&mut word, &sc);
                sc.bump();
                // A backslash with nothing after it to escape is the same
                // situation as a quote nobody closed: an escape someone
                // started typing and has not finished yet, not a literal
                // backslash of its own.
                match sc.bump() {
                    Some(escaped) => text.push(escaped),
                    None => complete = false,
                }
                word = Some((start, text));
            }
            ';' | '&' | '|' => {
                if let Some((start, text)) = word.take() {
                    tokens.push(Token {
                        start,
                        end: sc.pos,
                        inner_text: text,
                    });
                }
                let end = sc.pos;
                sc.bump();
                let op = match (c, sc.peek()) {
                    (';', _) => Operator::Semicolon,
                    ('&', Some(';')) => {
                        sc.bump();
                        Operator::AmpSemicolon
                    }
                    ('&', Some('&')) => {
                        sc.bump();
                        Operator::AndAnd
                    }
                    ('&', _) => Operator::Amp,
                    ('|', Some('&')) => {
                        sc.bump();
                        Operator::PipeAmp
                    }
                    ('|', Some('|')) => {
                        sc.bump();
                        Operator::OrOr
                    }
                    ('|', _) => Operator::Pipe,
                    _ => unreachable!("matched only on ';', '&' and '|'"),
                };
                commands.push(finish_command(
                    buffer,
                    cmd_start,
                    end,
                    std::mem::take(&mut tokens),
                    Some(op),
                ));
                cmd_start = sc.pos;
            }
            c => {
                let (start, mut text) = take_word(&mut word, &sc);
                text.push(c);
                sc.bump();
                word = Some((start, text));
            }
        }
    }
    if let Some((start, text)) = word.take() {
        tokens.push(Token {
            start,
            end: sc.pos,
            inner_text: text,
        });
    }
    commands.push(finish_command(buffer, cmd_start, sc.pos, tokens, None));

    Parse { commands, complete }
}

/// The command that contains byte offset `cursor`, or `None` when it falls on
/// an operator itself. A trailing space is part of the command it follows, so
/// a cursor sitting right after one lands on that command's own empty final
/// token rather than falling through.
pub fn command_at(buffer: &str, cursor: usize) -> Option<Command> {
    parse(buffer)
        .commands
        .into_iter()
        .find(|cmd| cmd.start <= cursor && cursor <= cmd.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(cmd: &Command) -> Vec<&str> {
        cmd.words.iter().map(|t| t.inner_text.as_str()).collect()
    }

    fn assignments(cmd: &Command) -> Vec<&str> {
        cmd.assignments
            .iter()
            .map(|t| t.inner_text.as_str())
            .collect()
    }

    #[test]
    fn a_bare_command_is_one_command_one_word() {
        let p = parse("cd work");
        assert_eq!(p.commands.len(), 1);
        assert!(p.complete);
        let cmd = &p.commands[0];
        assert_eq!((cmd.start, cmd.end), (0, 7));
        assert_eq!(words(cmd), vec!["cd", "work"]);
        assert_eq!(cmd.terminator, None);
    }

    #[test]
    fn each_operator_splits_the_line_in_two() {
        // No space around the operator, so the only thing under test is
        // which operator it reads as. A space in front of one earns its own
        // token under `a_trailing_space_adds_an_empty_final_token`.
        let cases: &[(&str, Operator)] = &[
            ("a;b", Operator::Semicolon),
            ("a&b", Operator::Amp),
            ("a&;b", Operator::AmpSemicolon),
            ("a|b", Operator::Pipe),
            ("a|&b", Operator::PipeAmp),
            ("a&&b", Operator::AndAnd),
            ("a||b", Operator::OrOr),
        ];
        for (line, op) in cases {
            let p = parse(line);
            assert_eq!(p.commands.len(), 2, "line: {line}");
            assert_eq!(words(&p.commands[0]), vec!["a"], "line: {line}");
            assert_eq!(words(&p.commands[1]), vec!["b"], "line: {line}");
            assert_eq!(p.commands[0].terminator, Some(*op), "line: {line}");
            assert_eq!(p.commands[1].terminator, None, "line: {line}");
        }
    }

    #[test]
    fn a_pipeline_binds_tighter_than_a_list() {
        // The plan asks only that this slice keep enough to tell the two
        // apart later: a run of `Pipe`/`PipeAmp` terminators is one pipeline
        // and a run of anything else chains pipelines into a list. Reading
        // the terminators back off is how a later pass would find that run.
        let p = parse("a|b;c|d&&e");
        let terminators: Vec<_> = p.commands.iter().map(|c| c.terminator).collect();
        assert_eq!(
            terminators,
            vec![
                Some(Operator::Pipe),
                Some(Operator::Semicolon),
                Some(Operator::Pipe),
                Some(Operator::AndAnd),
                None,
            ]
        );
        assert_eq!(
            p.commands.iter().map(words).collect::<Vec<_>>(),
            vec![vec!["a"], vec!["b"], vec!["c"], vec!["d"], vec!["e"]]
        );
    }

    #[test]
    fn an_operator_needs_no_surrounding_space() {
        let p = parse("a;b");
        assert_eq!(words(&p.commands[0]), vec!["a"]);
        assert_eq!(words(&p.commands[1]), vec!["b"]);
    }

    #[test]
    fn single_quotes_take_no_escapes() {
        let p = parse(r"'a\b'");
        assert_eq!(words(&p.commands[0]), vec![r"a\b"]);
        assert!(p.complete);
    }

    #[test]
    fn an_unterminated_single_quote_is_not_an_error() {
        let p = parse("'ab");
        assert!(!p.complete);
        assert_eq!(words(&p.commands[0]), vec!["ab"]);
    }

    #[test]
    fn double_quotes_escape_only_the_five_characters() {
        let p = parse(r#""\$\`\"\\ \n a""#.replace(r"\n", "\\\n").as_str());
        assert_eq!(words(&p.commands[0]), vec!["$`\"\\ \n a"]);
        assert!(p.complete);
    }

    #[test]
    fn a_backslash_before_an_ordinary_character_stays_literal_in_double_quotes() {
        let p = parse(r#""a\nb""#);
        assert_eq!(words(&p.commands[0]), vec![r"a\nb"]);
    }

    #[test]
    fn an_unterminated_double_quote_is_not_an_error() {
        let p = parse(r#""ab"#);
        assert!(!p.complete);
        assert_eq!(words(&p.commands[0]), vec!["ab"]);
    }

    #[test]
    fn dollar_quoted_text_is_taken_as_is() {
        let p = parse(r"$'a\nb'");
        assert_eq!(words(&p.commands[0]), vec![r"a\nb"]);
        assert!(p.complete);
    }

    #[test]
    fn an_unterminated_dollar_quote_is_not_an_error() {
        let p = parse(r"$'ab");
        assert!(!p.complete);
        assert_eq!(words(&p.commands[0]), vec!["ab"]);
    }

    #[test]
    fn a_backslash_outside_quotes_escapes_one_character() {
        let p = parse(r"a\ b");
        assert_eq!(words(&p.commands[0]), vec!["a b"]);
    }

    #[test]
    fn a_trailing_lone_backslash_is_not_complete() {
        // Nothing follows it to escape, which is the same open-ended state
        // as a quote nobody closed rather than a literal backslash.
        let p = parse(r"a\");
        assert!(!p.complete);
        assert_eq!(words(&p.commands[0]), vec!["a"]);
    }

    #[test]
    fn a_non_breaking_space_stays_inside_an_unquoted_word() {
        // `IFS` is space, tab and newline. Everything else is an ordinary
        // character of whatever word it sits in, non-breaking space included.
        let p = parse("cd a\u{a0}b");
        assert_eq!(words(&p.commands[0]), vec!["cd", "a\u{a0}b"]);
    }

    #[test]
    fn assignments_lead_the_command_and_a_lookalike_does_not_stop_it() {
        let cases: &[(&str, &[&str], &[&str])] = &[
            ("FOO=bar cmd arg", &["FOO=bar"], &["cmd", "arg"]),
            ("FOO+=bar cmd", &["FOO+=bar"], &["cmd"]),
            ("ARR[0]=x cmd", &["ARR[0]=x"], &["cmd"]),
            ("a=b=c cmd", &["a=b=c"], &["cmd"]),
        ];
        for (line, want_assignments, want_words) in cases {
            let p = parse(line);
            let cmd = &p.commands[0];
            assert_eq!(assignments(cmd), *want_assignments, "line: {line}");
            assert_eq!(words(cmd), *want_words, "line: {line}");
        }
    }

    #[test]
    fn a_character_the_gate_excludes_keeps_the_word_out_of_assignments() {
        let p = parse("FOO-BAR=x cmd");
        let cmd = &p.commands[0];
        assert!(assignments(cmd).is_empty());
        assert_eq!(words(cmd), vec!["FOO-BAR=x", "cmd"]);
    }

    #[test]
    fn a_trailing_space_adds_an_empty_final_token() {
        let p = parse("cd ");
        let cmd = &p.commands[0];
        assert_eq!(words(cmd), vec!["cd", ""]);
        assert_eq!(cmd.words[1].start, 3);
        assert_eq!(cmd.words[1].end, 3);
    }

    #[test]
    fn a_line_with_no_trailing_space_gets_no_empty_token() {
        let p = parse("cd wo");
        assert_eq!(words(&p.commands[0]), vec!["cd", "wo"]);
    }

    #[test]
    fn command_at_finds_the_command_holding_the_cursor() {
        // a(0) sp(1) ;(2) sp(3) b(4) sp(5) |(6) sp(7) c(8), length 9.
        // The space in front of an operator is trailing space for the
        // command it follows, so that command also carries the empty token
        // `a_trailing_space_adds_an_empty_final_token` describes.
        let line = "a ; b | c";
        assert_eq!(words(&command_at(line, 0).unwrap()), vec!["a", ""]); // start
        assert_eq!(words(&command_at(line, 2).unwrap()), vec!["a", ""]); // end
        assert_eq!(words(&command_at(line, 3).unwrap()), vec!["b", ""]); // start
        assert_eq!(words(&command_at(line, 4).unwrap()), vec!["b", ""]); // middle
        assert_eq!(words(&command_at(line, 6).unwrap()), vec!["b", ""]); // end
        assert_eq!(words(&command_at(line, 7).unwrap()), vec!["c"]); // start
        assert_eq!(words(&command_at(line, 8).unwrap()), vec!["c"]); // middle
        assert_eq!(words(&command_at(line, 9).unwrap()), vec!["c"]); // end
    }

    #[test]
    fn command_at_returns_none_on_the_operator_itself() {
        // A one-byte operator has no cursor position strictly inside it: `2`
        // in `a ; b` sits in front of the `;` and belongs to `a`'s own
        // trailing space instead. Only a two-byte operator has room for a
        // cursor that lands on neither command.
        assert!(command_at("a && b", 3).is_none());
        assert!(command_at("a |& b", 3).is_none());
    }

    #[test]
    fn command_at_reaches_inside_a_quoted_word() {
        let cmd = command_at("cd 'my docs'", 6).unwrap();
        assert_eq!(words(&cmd), vec!["cd", "my docs"]);
    }

    #[test]
    fn offsets_survive_a_multi_byte_character() {
        // "好" is three bytes. The word after it starts three bytes later
        // than a character count would suggest.
        let p = parse("cd 好/work");
        let cmd = &p.commands[0];
        assert_eq!(words(cmd), vec!["cd", "好/work"]);
        assert_eq!(cmd.words[1].start, 3);
        assert_eq!(cmd.words[1].end, 3 + "好/work".len());
    }

    #[test]
    fn offsets_survive_a_wide_emoji_character() {
        // "🎉" is four bytes and two terminal columns. Neither count is the
        // byte length this module has to use.
        let p = parse("echo 🎉 done");
        let cmd = &p.commands[0];
        assert_eq!(words(cmd), vec!["echo", "🎉", "done"]);
        let emoji = &cmd.words[1];
        assert_eq!(&"echo 🎉 done"[emoji.start..emoji.end], "🎉");
        let done = &cmd.words[2];
        assert_eq!(&"echo 🎉 done"[done.start..done.end], "done");
    }

    #[test]
    fn command_at_after_a_multi_byte_command_lands_on_the_right_one() {
        let line = "cd 好 ; ls";
        let semicolon_byte = line.find(';').unwrap();
        assert_eq!(
            words(&command_at(line, semicolon_byte - 1).unwrap()),
            vec!["cd", "好", ""]
        );
        assert_eq!(
            words(&command_at(line, semicolon_byte + 2).unwrap()),
            vec!["ls"]
        );
    }
}
