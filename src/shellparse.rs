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
//!
//! [`parse_with_aliases`] expands a command's own name when the alias table
//! names it. An alias value that itself holds an operator, such as
//! `alias upd='true && ls'`, is read as one command's words up to that
//! operator: turning one word into more than one command is the same
//! structural change the expansion family above is missing, and is left out
//! for the same reason. zsh also expands the *next* word when an alias value
//! ends in blank space, so that `alias sudo='sudo '` still expands whatever
//! follows it. This module does not do that either: it is a second read of
//! the following word rather than a property of the one being expanded, and
//! is left for whoever next needs it rather than left unrecorded.

use std::collections::{HashMap, HashSet};

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

/// A value ending in blank space reads as a word still being typed to
/// `parse`, which is right for a line someone is typing and wrong for an
/// alias value: that value is finished text, nobody is mid-word at its end.
/// The phantom token `parse` added for that case is therefore dropped here,
/// rather than spliced in as an argument nobody typed. It is the only zero-
/// width token `parse` ever produces, so that is what marks it.
fn without_trailing_empty(mut words: Vec<Token>) -> Vec<Token> {
    if words.last().is_some_and(|t| t.start == t.end) {
        words.pop();
    }
    words
}

/// Follow the alias chain starting at `word`, stopping once its name is not
/// in `aliases` or the cycle guard fires. `aliases` maps a name to its raw
/// value exactly as `${(kv)aliases}` would report it, quotes and all, so a
/// stripped copy is what actually replaces the word.
///
/// None of the expansion's own text exists in `buffer`: every token it
/// produces therefore carries `word`'s own byte range rather than one of its
/// own, and that is the only range a later insertion has to replace what the
/// person actually typed. The assignments and the words come back separately
/// because they land in different places: only `words` says what to keep
/// following the chain from, and only `words` takes `word`'s own place in the
/// command. An alias value that is nothing but assignments empties `words`
/// and the chain ends there with no command name at all, which is also what
/// the shell does with one.
fn expand_word(word: &Token, aliases: &HashMap<String, String>) -> (Vec<Token>, Vec<Token>) {
    let mut seen = HashSet::new();
    let mut assignments: Vec<Token> = Vec::new();
    let mut words: Vec<Token> = vec![word.clone()];
    while let Some(name) = words.first().map(|t| t.inner_text.clone()) {
        let Some(raw) = aliases.get(&name) else {
            break;
        };
        if !seen.insert(name) {
            break; // a name already opened in this chain: the cycle guard
        }
        let Some(command) = parse(&crate::shellword::unquote(raw))
            .commands
            .into_iter()
            .next()
        else {
            break;
        };
        if command.assignments.is_empty() && command.words.is_empty() {
            break;
        }
        assignments.extend(command.assignments);
        words = without_trailing_empty(command.words);
    }
    for token in assignments.iter_mut().chain(words.iter_mut()) {
        token.start = word.start;
        token.end = word.end;
    }
    (assignments, words)
}

/// `parse`, with `aliases` applied to each command's own name. An empty
/// table leaves every command exactly as `parse` returns it. Only a
/// command's name is a candidate: a leading assignment and every later
/// argument reach the shell as the person wrote them.
pub fn parse_with_aliases(buffer: &str, aliases: &HashMap<String, String>) -> Parse {
    let mut result = parse(buffer);
    for command in &mut result.commands {
        if let Some(name) = command.words.first() {
            let (assignments, words) = expand_word(name, aliases);
            command.assignments.splice(0..0, assignments);
            command.words.splice(0..1, words);
        }
    }
    result
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

    fn alias_table(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn an_alias_with_no_entry_leaves_the_command_alone() {
        let p = parse_with_aliases("cd work", &HashMap::new());
        assert_eq!(words(&p.commands[0]), vec!["cd", "work"]);
    }

    #[test]
    fn a_longer_alias_value_splices_in_and_a_real_argument_keeps_its_own_offset() {
        let aliases = alias_table(&[("ll", "ls -la")]);
        let p = parse_with_aliases("ll -h", &aliases);
        let cmd = &p.commands[0];
        assert_eq!(words(cmd), vec!["ls", "-la", "-h"]);
        // "ll" is bytes 0..2. Neither word the expansion invented exists in
        // the buffer, so both point back at the whole of what stood for them.
        assert_eq!((cmd.words[0].start, cmd.words[0].end), (0, 2));
        assert_eq!((cmd.words[1].start, cmd.words[1].end), (0, 2));
        // "-h" is untouched, real text: its own offset survives expansion.
        assert_eq!((cmd.words[2].start, cmd.words[2].end), (3, 5));
    }

    #[test]
    fn a_shorter_alias_value_still_maps_back_onto_the_original_word() {
        let aliases = alias_table(&[("quickcd", "cd")]);
        let p = parse_with_aliases("quickcd work", &aliases);
        let cmd = &p.commands[0];
        assert_eq!(words(cmd), vec!["cd", "work"]);
        // "quickcd" is bytes 0..7, longer than the "cd" that replaces it.
        assert_eq!((cmd.words[0].start, cmd.words[0].end), (0, 7));
        assert_eq!((cmd.words[1].start, cmd.words[1].end), (8, 12));
    }

    #[test]
    fn quotes_around_an_alias_value_are_stripped() {
        let aliases = alias_table(&[("gs", "'git status'")]);
        let p = parse_with_aliases("gs", &aliases);
        let cmd = &p.commands[0];
        assert_eq!(words(cmd), vec!["git", "status"]);
        assert_eq!((cmd.words[0].start, cmd.words[0].end), (0, 2));
        assert_eq!((cmd.words[1].start, cmd.words[1].end), (0, 2));
    }

    #[test]
    fn a_cycle_stops_instead_of_looping_forever() {
        let aliases = alias_table(&[("a", "b"), ("b", "a")]);
        let p = parse_with_aliases("a", &aliases);
        let cmd = &p.commands[0];
        // a -> b -> a, and the third hop repeats a name already opened in
        // this chain, so it stops there rather than going around again.
        assert_eq!(words(cmd), vec!["a"]);
        assert_eq!((cmd.words[0].start, cmd.words[0].end), (0, 1));
    }

    #[test]
    fn only_the_command_name_is_a_candidate_for_expansion() {
        let aliases = alias_table(&[("ls", "ls -la"), ("FOO", "should not run")]);
        let p = parse_with_aliases("FOO=ls ls", &aliases);
        let cmd = &p.commands[0];
        // "FOO" only looks like the alias name because it leads the line;
        // it is an assignment's name, not a word, and is never a candidate.
        assert_eq!(assignments(cmd), vec!["FOO=ls"]);
        assert_eq!(words(cmd), vec!["ls", "-la"]);
    }

    #[test]
    fn an_assignment_inside_an_alias_value_lands_in_assignments_not_words() {
        let aliases = alias_table(&[("e", "FOO=1 ls")]);
        let p = parse_with_aliases("e", &aliases);
        let cmd = &p.commands[0];
        assert_eq!(assignments(cmd), vec!["FOO=1"]);
        assert_eq!(words(cmd), vec!["ls"]);
        // Neither exists in the buffer on its own: both point back at "e".
        assert_eq!((cmd.assignments[0].start, cmd.assignments[0].end), (0, 1));
        assert_eq!((cmd.words[0].start, cmd.words[0].end), (0, 1));
    }

    #[test]
    fn a_trailing_space_in_an_alias_value_adds_no_phantom_argument() {
        let aliases = alias_table(&[("ll", "ls -la ")]);
        let p = parse_with_aliases("ll", &aliases);
        assert_eq!(words(&p.commands[0]), vec!["ls", "-la"]);
    }

    #[test]
    fn an_alias_that_is_assignments_alone_leaves_the_command_with_no_name() {
        // Also the case that has to stop the chain rather than loop: an
        // empty `words` fails the loop's own condition on the next pass.
        let aliases = alias_table(&[("e", "FOO=1")]);
        let p = parse_with_aliases("e", &aliases);
        let cmd = &p.commands[0];
        assert_eq!(assignments(cmd), vec!["FOO=1"]);
        assert!(words(cmd).is_empty());
    }

    #[test]
    fn an_assignment_and_a_trailing_space_in_the_same_alias_value_both_land_right() {
        let aliases = alias_table(&[("e", "FOO=1 ls ")]);
        let p = parse_with_aliases("e", &aliases);
        let cmd = &p.commands[0];
        assert_eq!(assignments(cmd), vec!["FOO=1"]);
        assert_eq!(words(cmd), vec!["ls"]);
    }
}
