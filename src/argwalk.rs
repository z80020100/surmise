//! Walks a parsed command's words against a loaded spec to say what a menu
//! should offer next.
//!
//! Two consumers run, in order, on every word after the command name except
//! the last: the subcommand consumer descends into a child of the current
//! node, and the option consumer reads the word against the node's own
//! `options`. The final word is never offered to either: it is read once,
//! kept as [`Walk::search_term`] and never used to advance the walk, because
//! it is the word a person is still typing rather than one they have
//! finished.
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
//! of the word as that argument's stuck-on value). `requires_equals` refuses
//! the plain exact-name match for an option that declares it, so only the
//! attached form reaches it. `--opt val` as two separate words needs no
//! special handling: the option consumer takes `--opt` as an exact name and
//! leaves `val` for the next word, same as any word nothing here recognizes.
//!
//! A bare `--` is not itself an option: the first one turns
//! [`Walk::end_of_options`] on, and a second one is read like any word
//! options are no longer being read from. `can_consume_options` also
//! refuses once `options_must_precede_arguments` is set and a word that was
//! not itself an option has already gone by. A word that fails every
//! consumer ends the walk when it looks like an option (a `-` word nothing
//! could read); otherwise it is treated as the non-option word
//! `options_must_precede_arguments` watches for, and the walk keeps going,
//! since a word shaped like a positional argument may still be one even
//! though nothing here can fill it in yet.
//!
//! Two consumers are still not here. An option-argument consumer and a
//! subcommand-argument consumer will fill [`Walk::current_arg`] from
//! `Opt::args` and `Subcommand::args`, once a word is consumed as the value
//! an option or the node itself asked for rather than as a name. Two more
//! `can_consume_options` gates wait on them: a pending mandatory or variadic
//! option argument, and `options_can_break_variadic_arg`. Variadic and
//! repeatable options, persistent options and `loadSpec` re-rooting all wait
//! on later slices too.

use crate::shellparse;
use crate::spec::{Arg, Opt, Separator, Subcommand};

/// What the walk found once it read every word but the last.
pub struct Walk<'a> {
    /// The node the walk ended on. Its own `subcommands` and `options` are
    /// what a menu reads to build its rows.
    pub node: &'a Subcommand,
    /// The argument a later consumer decided the final word fills. Always
    /// `None` until the option-argument and subcommand-argument consumers
    /// exist.
    pub current_arg: Option<Arg>,
    /// Every option the walk consumed. A menu drops one of these from
    /// `node.options` rather than offering it again.
    pub passed_options: Vec<&'a Opt>,
    /// The final word of the command, exactly as typed and never consumed.
    pub search_term: String,
    pub offers_subcommands: bool,
    pub offers_options: bool,
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
    let mut search_term = String::new();

    if words.len() > 1 {
        let last_index = words.len() - 1;
        for (index, word) in words.iter().enumerate().take(last_index).skip(1) {
            let text = word.inner_text.as_str();

            if let Some(child) = node.subcommands.get(text) {
                node = child.as_ref();
                command_index = index;
                continue;
            }

            if text == "--" && !end_of_options {
                end_of_options = true;
                continue;
            }

            if can_consume_options(node, end_of_options, seen_non_option) {
                match consume_option(node, text) {
                    Some(options) => {
                        passed_options.extend(options);
                        continue;
                    }
                    None if text.starts_with('-') => break,
                    None => {}
                }
            }

            seen_non_option = true;
        }
        search_term = words[last_index].inner_text.clone();
    }

    Walk {
        node,
        current_arg: None,
        passed_options,
        search_term,
        offers_subcommands: true,
        offers_options: can_consume_options(node, end_of_options, seen_non_option),
        offers_args: false,
        command_index,
        end_of_options,
    }
}

/// Whether the option consumer may still run at this point in the walk.
fn can_consume_options(node: &Subcommand, end_of_options: bool, seen_non_option: bool) -> bool {
    if end_of_options {
        return false;
    }
    let must_precede_arguments = node
        .parser_directives
        .as_ref()
        .is_some_and(|directives| directives.options_must_precede_arguments);
    !(must_precede_arguments && seen_non_option)
}

/// Tries every syntax the option consumer supports against one word, in the
/// order a person would expect to win.
fn consume_option<'a>(node: &'a Subcommand, text: &str) -> Option<Vec<&'a Opt>> {
    if let Some(opt) = node.options.get(text)
        && opt.requires_equals != Some(true)
    {
        return Some(vec![opt.as_ref()]);
    }

    if let Some(opt) = attached_option(node, text) {
        return Some(vec![opt]);
    }

    let posix_noncompliant = node
        .parser_directives
        .as_ref()
        .is_some_and(|directives| directives.flags_are_posix_noncompliant);
    if !posix_noncompliant && text.starts_with('-') && !text.starts_with("--") {
        return short_chain(node, text);
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
/// an argument ends the chain there, with whatever follows read as its
/// stuck-on value — which is also how a single `-ovalue` word is read, as a
/// chain that happens to be one letter long.
fn short_chain<'a>(node: &'a Subcommand, text: &str) -> Option<Vec<&'a Opt>> {
    if text.len() <= 1 {
        return None; // a lone `-`: nothing to chain
    }
    let mut consumed = Vec::new();
    for letter in text[1..].chars() {
        let opt = node.options.get(&format!("-{letter}"))?;
        consumed.push(opt.as_ref());
        if !opt.args.is_empty() {
            break;
        }
    }
    Some(consumed)
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
    }

    #[test]
    fn a_partial_word_after_a_subcommand_does_not_descend_again() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git switch ma"), &git);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
        assert_eq!(walk.search_term, "ma");
        assert_eq!(walk.command_index, 1);
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

    #[test]
    fn a_second_double_dash_is_read_as_an_ordinary_word() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git -- -- switch "), &git);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
        assert!(walk.end_of_options);
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

    /// No committed spec exercises `option_arg_separators` with more than one
    /// separator on an option that names none of its own — `esbuild`, which
    /// configures two, gives every argument-taking option its own explicit
    /// one instead — and none pairs an explicit separator with a spec-wide
    /// list that does not already contain it. This node is built by hand to
    /// cover both: `--level` proves an explicit separator excludes the
    /// others, and `--mode` proves an option naming none of its own tries
    /// every separator the spec configures.
    #[test]
    fn option_separators_combine_a_per_option_override_with_the_spec_wide_list() {
        let mut options = HashMap::new();
        options.insert(
            "--level".to_string(),
            Rc::new(bare_opt(
                "--level",
                Some(Separator::Explicit(":".to_string())),
            )),
        );
        options.insert("--mode".to_string(), Rc::new(bare_opt("--mode", None)));
        let node = bare_node(
            options,
            ParserDirectives {
                option_arg_separators: vec!["=".to_string(), ":".to_string()],
                ..Default::default()
            },
        );

        let via_colon = walk(&command("probe --level:5 --mode:fast "), &node);
        assert_eq!(via_colon.passed_options.len(), 2);

        let via_equals = walk(&command("probe --mode=fast "), &node);
        assert_eq!(via_equals.passed_options.len(), 1);

        let level_rejects_equals = walk(&command("probe --level=5 "), &node);
        assert!(level_rejects_equals.passed_options.is_empty());
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
