//! Walks a parsed command's words against a loaded spec to say what a menu
//! should offer next.
//!
//! Four consumers run, in this order, on every word after the command name
//! except the last: the subcommand consumer descends into a child of the
//! current node; the option consumer reads the word against the node's own
//! `options`; the option-argument consumer fills the args an already-
//! consumed option declared; the subcommand-argument consumer fills the
//! current node's own `args`. The final word is never offered to any of
//! them: it is read once, kept as [`Walk::search_term`] and never used to
//! advance the walk, because it is the word a person is still typing rather
//! than one they have finished.
//!
//! The option consumer tries, per word: an exact name first, covering
//! `--opt` and `-o` alike, and — when `flags_are_posix_noncompliant` is set
//! — a whole `-abc`-shaped token the spec declares literally; then a long
//! option with its value stuck on behind a separator, `--opt=val`, tried on
//! whichever separators `parser_directives.option_arg_separators` or the
//! option's own `requires_separator` name, `=` failing both; then, unless
//! `flags_are_posix_noncompliant` says every dash-word is a whole name of
//! its own, a short option or a left-to-right chain of them (`-abc`, and
//! `-ovalue` as the one-letter case of the same chain: the first letter
//! whose option takes an argument ends the chain there and leaves the rest
//! of the word as that argument's stuck-on value, filling its first
//! argument the same way an attached `=val` does). `requires_equals`
//! refuses the plain exact-name match for an option that declares it, so
//! only the attached form reaches it. `is_repeatable` gates every one of
//! these matches: [`Walk::passed_options`] keeps one entry per use rather
//! than deduplicating, so counting a candidate's own occurrences in it
//! against `Repeatable::Once`, `Unlimited` or `Times(n)` is what tells an
//! already-used-up option from one that may be used again.
//!
//! A bare `--` is not itself an option: the first one turns
//! [`Walk::end_of_options`] on, and a second one is read like any word
//! options are no longer being read from. A node that declares `--` as a
//! real option of its own — `git diff --`, with its own trailing path
//! argument — reaches that through the ordinary option consumer instead,
//! since the exact-name match is tried before this fallback ever runs.
//!
//! **Option arguments.** An option that declares `args` leaves them pending
//! after it is consumed, in order. A pending argument that is not
//! `is_optional` and has not yet taken a word forces the very next word to
//! fill it, whatever else that word looks like, ahead of the subcommand and
//! option consumers entirely; an `is_optional` argument does not force
//! anything, so a subcommand or another option can still win the word.
//! `is_variadic` keeps a filled argument as the pending one rather than
//! advancing, so it goes on taking words until something else claims one.
//! `can_consume_options` refuses new options while a pending argument is
//! both filled and variadic, unless that argument's own
//! `options_can_break_variadic_arg` allows the interruption (the default,
//! when it names neither, is to allow it); `git diff --`'s own path
//! argument sets it `false` specifically to close that door once its `--`
//! has been read, since no shell-level `--` is involved to do it instead.
//!
//! **Subcommand arguments.** A word every other consumer refuses fills the
//! current node's own `args`, in order, with the same `is_optional` and
//! `is_variadic` handling as an option's — except that nothing forces one:
//! it is already the last consumer tried. The first word actually consumed
//! this way turns [`Walk::offers_subcommands`] off for the rest of this
//! node's words (`entered_subcommand_args`), which is what keeps an
//! optional argument from being mistaken for a place a subcommand could
//! still appear once something has started filling it.
//!
//! **The final word still only annotates.** Whatever argument is pending
//! when the loop stops — an option's own, or the current node's — becomes
//! [`Walk::current_arg`] and turns [`Walk::offers_args`] on, without being
//! consumed or advancing anything. A forced pending argument also turns
//! `offers_subcommands` and `offers_options` off for that word, since
//! nothing else could win it either.
//!
//! `loadSpec` re-rooting and persistent options are not here yet.

use crate::shellparse;
use crate::spec::{Arg, Opt, Repeatable, Separator, Subcommand};

/// An option's own arguments, mid-consumption: the option, the index of the
/// next of its `args` to fill, and whether that one has already taken a
/// word.
type OptionArg<'a> = (&'a Opt, usize, bool);

/// What the walk found once it read every word but the last.
pub struct Walk<'a> {
    /// The node the walk ended on. Its own `subcommands` and `options` are
    /// what a menu reads to build its rows.
    pub node: &'a Subcommand,
    /// The argument the final word would fill, if any is pending: an
    /// option's own, or the current node's.
    pub current_arg: Option<Arg>,
    /// Every option the walk consumed, one entry per use. A menu drops one
    /// already at its `is_repeatable` cap rather than offering it again.
    pub passed_options: Vec<&'a Opt>,
    /// The final word of the command, exactly as typed and never consumed.
    pub search_term: String,
    pub offers_subcommands: bool,
    pub offers_options: bool,
    /// Whether [`Walk::current_arg`] is set.
    pub offers_args: bool,
    /// Index into the command's `words` of the word that named `node`. `0`
    /// means the walk never left the command name itself.
    pub command_index: usize,
    /// Whether a bare `--` has already been read.
    pub end_of_options: bool,
}

/// Walks `command`'s words against `root`, one word at a time.
///
/// `command.words[0]` is the command name that resolved to `root`; the walk
/// itself starts on `command.words[1]`. The caller loads `root` beforehand,
/// with [`crate::spec::load`] or otherwise, so this function does no I/O.
pub fn walk<'a>(command: &shellparse::Command, root: &'a Subcommand) -> Walk<'a> {
    let words = &command.words;
    let mut node = root;
    let mut command_index = 0;
    let mut end_of_options = false;
    let mut seen_non_option = false;
    let mut passed_options: Vec<&'a Opt> = Vec::new();
    let mut option_arg: Option<OptionArg<'a>> = None;
    let mut entered_subcommand_args = false;
    let mut subcommand_arg_index = 0;
    let mut subcommand_arg_filled = false;
    let mut search_term = String::new();

    if words.len() > 1 {
        let last_index = words.len() - 1;
        for (index, word) in words.iter().enumerate().take(last_index).skip(1) {
            let text = word.inner_text.as_str();

            // The option-argument consumer, forced: a pending argument that
            // still needs its first word and is not optional takes this
            // word no matter what it looks like.
            if let Some((opt, arg_index, filled)) = option_arg
                && !filled
                && !opt.args[arg_index].is_optional.unwrap_or(false)
            {
                option_arg = advance_option_arg(opt, arg_index);
                continue;
            }

            if !entered_subcommand_args && let Some(child) = node.subcommands.get(text) {
                node = child.as_ref();
                command_index = index;
                entered_subcommand_args = false;
                subcommand_arg_index = 0;
                subcommand_arg_filled = false;
                option_arg = None;
                continue;
            }

            let variadic = active_variadic(
                node,
                option_arg,
                entered_subcommand_args,
                subcommand_arg_index,
                subcommand_arg_filled,
            );
            if can_consume_options(node, end_of_options, seen_non_option, variadic) {
                match consume_option(node, text, &passed_options) {
                    Some((opts, pending)) => {
                        passed_options.extend(opts);
                        option_arg = pending;
                        continue;
                    }
                    None if text == "--" && !end_of_options => {
                        end_of_options = true;
                        continue;
                    }
                    None if text.starts_with('-') => break,
                    None => {}
                }
            }

            // The option-argument consumer, residual: a pending argument
            // that is optional, or already filled and merely continuing a
            // variadic run, takes whatever nothing else wanted.
            if let Some((opt, arg_index, _)) = option_arg {
                option_arg = advance_option_arg(opt, arg_index);
                continue;
            }

            // The subcommand-argument consumer: the last resort, filling
            // the current node's own `args` in order.
            if subcommand_arg_index < node.args.len() {
                if node.args[subcommand_arg_index].is_variadic == Some(true) {
                    subcommand_arg_filled = true;
                } else {
                    subcommand_arg_index += 1;
                    subcommand_arg_filled = false;
                }
                entered_subcommand_args = true;
                seen_non_option = true;
                continue;
            }

            seen_non_option = true;
        }
        search_term = words[last_index].inner_text.clone();
    }

    let forced = matches!(
        option_arg,
        Some((opt, arg_index, filled))
            if !filled && !opt.args[arg_index].is_optional.unwrap_or(false)
    );
    let current_arg = if let Some((opt, arg_index, _)) = option_arg {
        Some(opt.args[arg_index].clone())
    } else if subcommand_arg_index < node.args.len() {
        Some(node.args[subcommand_arg_index].clone())
    } else {
        None
    };
    let variadic = active_variadic(
        node,
        option_arg,
        entered_subcommand_args,
        subcommand_arg_index,
        subcommand_arg_filled,
    );

    Walk {
        node,
        offers_args: current_arg.is_some(),
        current_arg,
        passed_options,
        search_term,
        offers_subcommands: !entered_subcommand_args && !forced,
        offers_options: !forced
            && can_consume_options(node, end_of_options, seen_non_option, variadic),
        command_index,
        end_of_options,
    }
}

/// Whether the option consumer may still run at this point in the walk.
fn can_consume_options(
    node: &Subcommand,
    end_of_options: bool,
    seen_non_option: bool,
    active_variadic: Option<&Arg>,
) -> bool {
    if end_of_options {
        return false;
    }
    if let Some(arg) = active_variadic
        && !arg.options_can_break_variadic_arg.unwrap_or(true)
    {
        return false;
    }
    let must_precede_arguments = node
        .parser_directives
        .as_ref()
        .is_some_and(|directives| directives.options_must_precede_arguments);
    !(must_precede_arguments && seen_non_option)
}

/// The variadic argument currently receiving words, if any: an option's own,
/// once it has taken at least one; otherwise the current node's own, once
/// entered. Neither counts before its first word, because nothing is
/// "in progress" for an option consumer to be protected from yet.
fn active_variadic<'a>(
    node: &'a Subcommand,
    option_arg: Option<OptionArg<'a>>,
    entered_subcommand_args: bool,
    subcommand_arg_index: usize,
    subcommand_arg_filled: bool,
) -> Option<&'a Arg> {
    if let Some((opt, arg_index, filled)) = option_arg {
        let arg = &opt.args[arg_index];
        return (filled && arg.is_variadic == Some(true)).then_some(arg);
    }
    if entered_subcommand_args && subcommand_arg_filled && subcommand_arg_index < node.args.len() {
        let arg = &node.args[subcommand_arg_index];
        if arg.is_variadic == Some(true) {
            return Some(arg);
        }
    }
    None
}

/// Moves a just-filled option argument on: a variadic one stays put, marked
/// filled, so it keeps taking words; anything else moves to the next
/// argument, or to no pending argument at all past the last one.
fn advance_option_arg<'a>(opt: &'a Opt, arg_index: usize) -> Option<OptionArg<'a>> {
    if opt.args[arg_index].is_variadic == Some(true) {
        Some((opt, arg_index, true))
    } else {
        start_pending_from(opt, arg_index + 1)
    }
}

/// The pending state a freshly consumed `opt` starts in, from its argument
/// at `index` onward — `1` for an option whose value was already absorbed
/// inline (an attached `=val` or a chain's stuck-on value), `0` otherwise.
fn start_pending_from<'a>(opt: &'a Opt, index: usize) -> Option<OptionArg<'a>> {
    (index < opt.args.len()).then_some((opt, index, false))
}

fn start_pending<'a>(opt: &'a Opt) -> Option<OptionArg<'a>> {
    start_pending_from(opt, 0)
}

/// Whether `opt` has not yet reached its own `is_repeatable` cap, counting
/// its occurrences in `passed` by identity rather than by name, since two
/// entries can share every alias.
fn is_available(opt: &Opt, passed: &[&Opt]) -> bool {
    let mut used = 0usize;
    for other in passed {
        if std::ptr::eq(*other, opt) {
            used += 1;
        }
    }
    match opt.is_repeatable {
        Repeatable::Once => used < 1,
        Repeatable::Unlimited => true,
        Repeatable::Times(cap) => used < cap as usize,
    }
}

/// Tries every syntax the option consumer supports against one word, in the
/// order a person would expect to win. The second element of a successful
/// result is the pending-argument state the caller should adopt.
fn consume_option<'a>(
    node: &'a Subcommand,
    text: &str,
    passed: &[&'a Opt],
) -> Option<(Vec<&'a Opt>, Option<OptionArg<'a>>)> {
    if let Some(opt) = node.options.get(text)
        && opt.requires_equals != Some(true)
        && is_available(opt, passed)
    {
        let opt = opt.as_ref();
        return Some((vec![opt], start_pending(opt)));
    }

    if let Some(opt) = attached_option(node, text)
        && is_available(opt, passed)
    {
        return Some((vec![opt], start_pending_from(opt, 1)));
    }

    let posix_noncompliant = node
        .parser_directives
        .as_ref()
        .is_some_and(|directives| directives.flags_are_posix_noncompliant);
    if !posix_noncompliant && text.starts_with('-') && !text.starts_with("--") {
        return short_chain(node, text, passed);
    }

    None
}

/// A long option's value stuck to its name behind a separator, such as
/// `--format=json`. Each option is tried against its own resolved
/// separators: just the one `requires_separator` names when it names one,
/// every separator `option_arg_separators` lists when it does not, and `=`
/// alone when neither the option nor the spec names any.
fn attached_option<'a>(node: &'a Subcommand, text: &str) -> Option<&'a Opt> {
    node.options.values().find_map(|opt| {
        let name = opt
            .name
            .iter()
            .find(|name| text.starts_with(name.as_str()))?;
        let rest = &text[name.len()..];
        candidate_separators(node, opt)
            .iter()
            .any(|separator| rest.starts_with(separator.as_str()))
            .then(|| opt.as_ref())
    })
}

/// The separator or separators an attached value may use for `opt`, per the
/// resolution `Opt::requires_separator`'s own doc comment gives: an explicit
/// string is used alone; `Default` resolves to the spec's first configured
/// separator, or `=`; declaring neither falls back to every separator the
/// spec configures, or `=` when it configures none.
fn candidate_separators(node: &Subcommand, opt: &Opt) -> Vec<String> {
    let configured = node
        .parser_directives
        .as_ref()
        .map(|directives| directives.option_arg_separators.clone())
        .unwrap_or_default();

    match &opt.requires_separator {
        Some(Separator::Explicit(separator)) => vec![separator.clone()],
        Some(Separator::Default) => {
            vec![
                configured
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "=".to_string()),
            ]
        }
        None if configured.is_empty() => vec!["=".to_string()],
        None => configured,
    }
}

/// A run of one-letter short options packed into one word, such as `-nvf`.
/// Consumption reads left to right: the first letter whose own option takes
/// an argument ends the chain there, with whatever follows read as that
/// argument's stuck-on value — which is also how a single `-ovalue` word is
/// read, as a chain that happens to be one letter long.
fn short_chain<'a>(
    node: &'a Subcommand,
    text: &str,
    passed: &[&'a Opt],
) -> Option<(Vec<&'a Opt>, Option<OptionArg<'a>>)> {
    if text.len() <= 1 {
        return None; // a lone `-`: nothing to chain
    }
    let mut consumed = Vec::new();
    for (offset, letter) in text[1..].char_indices() {
        let opt = node.options.get(&format!("-{letter}"))?.as_ref();
        if !is_available(opt, passed) {
            return None;
        }
        consumed.push(opt);
        if !opt.args.is_empty() {
            let consumed_len = 1 + offset + letter.len_utf8();
            let pending = if text.len() > consumed_len {
                start_pending_from(opt, 1) // the rest of the word is its value
            } else {
                start_pending(opt) // nothing stuck on: the value is still to come
            };
            return Some((consumed, pending));
        }
    }
    Some((consumed, None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{self, ParserDirectives, Repeatable};
    use std::collections::HashMap;
    use std::rc::Rc;

    fn command(line: &str) -> shellparse::Command {
        shellparse::parse(line).commands.into_iter().next().unwrap()
    }

    #[test]
    fn a_trailing_space_offers_the_root_subcommands() {
        let git = spec::load("git", &[]).unwrap();
        let walk = walk(&command("git "), &git);
        assert!(std::ptr::eq(walk.node, &git));
        assert_eq!(walk.search_term, "");
        assert!(walk.offers_subcommands);
    }

    #[test]
    fn a_partial_word_is_the_search_term_and_does_not_descend() {
        let git = spec::load("git", &[]).unwrap();
        let walk = walk(&command("git swi"), &git);
        assert!(std::ptr::eq(walk.node, &git));
        assert_eq!(walk.search_term, "swi");
        assert!(walk.offers_subcommands);
    }

    #[test]
    fn a_full_subcommand_and_trailing_space_descends() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git switch "), &git);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
        assert_eq!(walk.search_term, "");
        assert_eq!(walk.command_index, 1);
        // A trailing space right after the subcommand means its own first
        // argument is current, even though nothing has been typed for it yet.
        assert_eq!(walk.current_arg.unwrap().name, switch.args[0].name);
        assert!(walk.offers_args);
    }

    #[test]
    fn a_partial_word_after_a_subcommand_does_not_descend_again() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git switch ma"), &git);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
        assert_eq!(walk.search_term, "ma");
        assert_eq!(walk.command_index, 1);
        assert_eq!(walk.current_arg.unwrap().name, switch.args[0].name);
    }

    #[test]
    fn two_subcommands_descend_twice() {
        let docker = spec::load("docker", &[]).unwrap();
        let container = docker.subcommands.get("container").unwrap();
        let ls = container.subcommands.get("ls").unwrap();
        let walk = walk(&command("docker container ls "), &docker);
        assert!(std::ptr::eq(walk.node, ls.as_ref()));
        assert_eq!(walk.search_term, "");
        assert_eq!(walk.command_index, 2);
    }

    #[test]
    fn a_word_that_names_no_subcommand_stops_the_walk_where_it_stopped() {
        let git = spec::load("git", &[]).unwrap();
        let walk = walk(&command("git nosuchcommand extra "), &git);
        assert!(std::ptr::eq(walk.node, &git));
        assert_eq!(walk.command_index, 0);
        assert_eq!(walk.search_term, "");
    }

    #[test]
    fn a_bare_long_option_is_taken_even_though_its_value_is_a_separate_word() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let create = switch.options.get("--create").unwrap();
        let walk = walk(&command("git switch --create newbranch "), &git);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], create.as_ref()));
        assert_eq!(walk.search_term, "");
        // `newbranch` filled the mandatory `new branch` argument, so the
        // optional `start point` that follows it is now the pending one.
        assert_eq!(walk.current_arg.unwrap().name, create.args[1].name);
    }

    #[test]
    fn a_chain_of_short_options_consumes_each_one() {
        let git = spec::load("git", &[]).unwrap();
        let add = git.subcommands.get("add").unwrap();
        let dry_run = add.options.get("-n").unwrap();
        let verbose = add.options.get("-v").unwrap();
        let force = add.options.get("-f").unwrap();
        let walk = walk(&command("git add -nvf "), &git);
        assert_eq!(walk.passed_options.len(), 3);
        assert!(std::ptr::eq(walk.passed_options[0], dry_run.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[1], verbose.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[2], force.as_ref()));
    }

    #[test]
    fn a_short_options_stuck_on_value_does_not_chain_further() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let create = switch.options.get("-c").unwrap();
        let walk = walk(&command("git switch -cnewbranch "), &git);
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], create.as_ref()));
    }

    #[test]
    fn a_double_dash_mid_line_ends_options_without_blocking_a_descend() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git -- switch "), &git);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
        assert!(walk.end_of_options);
        assert_eq!(walk.search_term, "");
    }

    /// `git`'s own root declares an optional `alias` argument alongside its
    /// subcommands. Once options are off, a second `--` has nothing left to
    /// mean except that argument, and filling it is what closes subcommands
    /// off for every word after it — `switch` is a perfectly real
    /// subcommand name and still does not reach the subcommand consumer.
    #[test]
    fn a_word_that_fills_the_roots_own_argument_closes_off_subcommands() {
        let git = spec::load("git", &[]).unwrap();
        assert!(!git.args.is_empty());
        let walk = walk(&command("git -- -- switch "), &git);
        assert!(walk.end_of_options);
        assert!(std::ptr::eq(walk.node, &git));
        assert!(!walk.offers_subcommands);
    }

    #[test]
    fn a_double_dash_as_the_final_word_is_only_the_search_term() {
        let git = spec::load("git", &[]).unwrap();
        let walk = walk(&command("git --"), &git);
        assert!(std::ptr::eq(walk.node, &git));
        assert_eq!(walk.search_term, "--");
        assert!(!walk.end_of_options);
        assert!(walk.passed_options.is_empty());
    }

    #[test]
    fn options_must_precede_arguments_refuses_an_option_once_a_plain_word_has_gone_by() {
        let fold = spec::load("fold", &[]).unwrap();
        let walk = walk(&command("fold somefile -b "), &fold);
        assert!(walk.passed_options.is_empty());
    }

    #[test]
    fn options_still_consume_fine_when_no_plain_word_has_gone_by() {
        let fold = spec::load("fold", &[]).unwrap();
        let b = fold.options.get("-b").unwrap();
        let s = fold.options.get("-s").unwrap();
        let walk = walk(&command("fold -b -s "), &fold);
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], b.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[1], s.as_ref()));
    }

    #[test]
    fn each_options_own_explicit_separator_is_honored() {
        let esbuild = spec::load("esbuild", &[]).unwrap();
        let loader = esbuild.options.get("--loader").unwrap();
        let format = esbuild.options.get("--format").unwrap();
        let walk = walk(&command("esbuild --loader:js --format=esm "), &esbuild);
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], loader.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[1], format.as_ref()));
    }

    #[test]
    fn requires_equals_refuses_the_bare_form() {
        let mosh = spec::load("mosh", &[]).unwrap();
        let walk = walk(&command("mosh --predict --family=inet "), &mosh);
        assert!(walk.passed_options.is_empty());
        assert!(std::ptr::eq(walk.node, &mosh));
        assert_eq!(walk.command_index, 0);
    }

    #[test]
    fn requires_equals_accepts_the_attached_form() {
        let mosh = spec::load("mosh", &[]).unwrap();
        let predict = mosh.options.get("--predict").unwrap();
        let walk = walk(&command("mosh --predict=always --family=inet "), &mosh);
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], predict.as_ref()));
    }

    #[test]
    fn requires_separator_default_resolves_to_the_spec_wide_fallback() {
        let ua = spec::load("ua", &[]).unwrap();
        let status = ua.subcommands.get("status").unwrap();
        let format = status.options.get("--format").unwrap();
        let walk = walk(&command("ua status --format=json "), &ua);
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], format.as_ref()));
    }

    #[test]
    fn requires_separator_explicit_names_its_own_separator() {
        let eza = spec::load("eza", &[]).unwrap();
        let color_scale = eza.options.get("--color-scale").unwrap();
        let walk = walk(&command("eza --color-scale=all --color=auto "), &eza);
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], color_scale.as_ref()));
    }

    /// `nextflow`'s root configures two separators, `=` and `.` — the
    /// second because the same corpus has options literally named
    /// `-e.`/`-process.` that take `key=value`, not because `.` is a typo
    /// for `=`. Its `run` subcommand has plenty of options, like `-profile`
    /// and `-w`, that declare no `requiresSeparator` of their own, so they
    /// fall back to trying every separator the root configures.
    #[test]
    fn option_arg_separators_tries_every_configured_separator() {
        let nextflow = spec::load("nextflow", &[]).unwrap();
        let run = nextflow.subcommands.get("run").unwrap();
        let profile = run.options.get("-profile").unwrap();
        let work_dir = run.options.get("-w").unwrap();
        let walk = walk(
            &command("nextflow run -profile=docker -w.testdir "),
            &nextflow,
        );
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], profile.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[1], work_dir.as_ref()));
    }

    /// No committed spec pairs an explicit `requiresSeparator` with a
    /// spec-wide `option_arg_separators` list that names a different
    /// separator, so this node is built by hand to prove the override
    /// excludes the spec-wide list rather than merely joining it.
    #[test]
    fn requires_separator_explicit_overrides_the_spec_wide_list() {
        let mut options = HashMap::new();
        options.insert(
            "--level".to_string(),
            Rc::new(bare_opt(
                "--level",
                Some(Separator::Explicit(":".to_string())),
            )),
        );
        let node = bare_node(
            options,
            ParserDirectives {
                option_arg_separators: vec!["=".to_string()],
                ..Default::default()
            },
        );

        let via_colon = walk(&command("probe --level:5 "), &node);
        assert_eq!(via_colon.passed_options.len(), 1);

        let via_equals = walk(&command("probe --level=5 "), &node);
        assert!(via_equals.passed_options.is_empty());
    }

    #[test]
    fn flags_are_posix_noncompliant_refuses_to_decompose_a_chain() {
        let kubectx = spec::load("kubectx", &[]).unwrap();
        let walk = walk(&command("kubectx -hc "), &kubectx);
        assert!(walk.passed_options.is_empty());
        assert!(std::ptr::eq(walk.node, &kubectx));
    }

    #[test]
    fn a_final_word_that_looks_like_an_option_only_annotates() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git switch --c"), &git);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
        assert_eq!(walk.search_term, "--c");
        assert!(walk.offers_options);
        assert!(walk.passed_options.is_empty());
    }

    #[test]
    fn a_final_word_that_looks_like_an_option_does_not_offer_options_once_they_are_off() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git switch -- --c"), &git);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
        assert_eq!(walk.search_term, "--c");
        assert!(walk.end_of_options);
        assert!(!walk.offers_options);
    }

    #[test]
    fn a_mandatory_option_argument_is_forced_even_when_the_word_looks_like_an_option() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let create = switch.options.get("--create").unwrap();
        let walk = walk(&command("git switch --create -weirdname "), &git);
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], create.as_ref()));
        // `-weirdname` was forced into `new branch`, not attempted as an
        // option, so the pending argument has moved on to `start point`.
        assert_eq!(walk.current_arg.unwrap().name, create.args[1].name);
    }

    /// `git`'s own root declares subcommands and an optional `alias`
    /// argument side by side. The subcommand consumer still runs first, so
    /// a real subcommand name is never mistaken for that argument.
    #[test]
    fn an_optional_root_argument_does_not_block_a_subcommand() {
        let git = spec::load("git", &[]).unwrap();
        assert!(!git.args.is_empty());
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git switch "), &git);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
    }

    #[test]
    fn a_variadic_subcommand_argument_takes_several_words() {
        let git = spec::load("git", &[]).unwrap();
        let add = git.subcommands.get("add").unwrap();
        let walk = walk(&command("git add file1 file2 file3 "), &git);
        assert!(std::ptr::eq(walk.node, add.as_ref()));
        assert_eq!(walk.current_arg.unwrap().name, add.args[0].name);
        assert!(walk.offers_args);
    }

    #[test]
    fn option_repeatable_once_refuses_a_second_occurrence() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let detach = switch.options.get("-d").unwrap();
        let walk = walk(&command("git switch -d -d "), &git);
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], detach.as_ref()));
    }

    #[test]
    fn option_repeatable_times_caps_at_its_count() {
        let git = spec::load("git", &[]).unwrap();
        let branch = git.subcommands.get("branch").unwrap();
        let verbose = branch.options.get("-v").unwrap();
        let walk = walk(&command("git branch -v -v -v "), &git);
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], verbose.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[1], verbose.as_ref()));
    }

    #[test]
    fn option_repeatable_unlimited_has_no_cap() {
        let git = spec::load("git", &[]).unwrap();
        let rebase = git.subcommands.get("rebase").unwrap();
        let strategy = rebase.options.get("-s").unwrap();
        let walk = walk(
            &command("git rebase -s resolve -s recursive -s ours "),
            &git,
        );
        assert_eq!(walk.passed_options.len(), 3);
        for passed in &walk.passed_options {
            assert!(std::ptr::eq(*passed, strategy.as_ref()));
        }
    }

    #[test]
    fn an_option_interrupts_a_variadic_argument_by_default() {
        let git = spec::load("git", &[]).unwrap();
        let add = git.subcommands.get("add").unwrap();
        let dry_run = add.options.get("-n").unwrap();
        let walk = walk(&command("git add file1 -n file2 "), &git);
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], dry_run.as_ref()));
        assert!(std::ptr::eq(walk.node, add.as_ref()));
        assert!(walk.offers_args); // the pathspec keeps taking words afterward
    }

    /// `git diff --`'s own trailing path argument sets
    /// `optionsCanBreakVariadicArg: false`, so once it is filling, a word
    /// that would otherwise be a perfectly good option is instead read as
    /// another path.
    #[test]
    fn options_can_break_variadic_arg_false_refuses_the_interruption() {
        let git = spec::load("git", &[]).unwrap();
        let diff = git.subcommands.get("diff").unwrap();
        let dashdash = diff.options.get("--").unwrap();
        let walk = walk(&command("git diff -- file1 --staged file2 "), &git);
        // Only `--` itself was consumed as an option; `--staged` was read as
        // another path instead of interrupting the variadic one.
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], dashdash.as_ref()));
        assert!(std::ptr::eq(walk.node, diff.as_ref()));
        assert!(walk.offers_args);
    }

    fn bare_opt(name: &str, requires_separator: Option<Separator>) -> Opt {
        Opt {
            name: vec![name.to_string()],
            args: Vec::new(),
            is_persistent: false,
            is_required: None,
            description: None,
            icon: None,
            priority: None,
            hidden: None,
            display_name: None,
            insert_value: None,
            is_repeatable: Repeatable::default(),
            requires_separator,
            requires_equals: None,
            exclusive_on: Vec::new(),
            depends_on: Vec::new(),
            is_dangerous: None,
            extra: serde_json::Map::new(),
        }
    }

    fn bare_node(options: HashMap<String, Rc<Opt>>, directives: ParserDirectives) -> Subcommand {
        Subcommand {
            name: vec!["probe".to_string()],
            subcommands: HashMap::new(),
            options,
            persistent_options: HashMap::new(),
            args: Vec::new(),
            parser_directives: Some(directives),
            load_spec: Vec::new(),
            cache: None,
            description: None,
            icon: None,
            priority: None,
            hidden: None,
            display_name: None,
            insert_value: None,
            requires_subcommand: None,
            additional_suggestions: Vec::new(),
            is_dangerous: None,
            filter_strategy: None,
            extra: serde_json::Map::new(),
        }
    }
}
