//! Walks a parsed command's words against a loaded spec to say what a menu
//! should offer next.
//!
//! Four consumers run, in this order, on every word after the command name
//! except the last: the subcommand consumer descends into a child of the
//! current node; the option consumer reads the word against the node's own
//! `options` and every ancestor's `persistent_options`; the option-argument
//! consumer fills the args an already-consumed option declared; the
//! subcommand-argument consumer fills the current node's own `args`. The
//! final word is never offered to any of them: it is read once, kept as
//! [`Walk::search_term`] and never used to advance the walk, because it is
//! the word a person is still typing rather than one they have finished.
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
//! **Persistent options.** `Subcommand::persistent_options` is a second map
//! beside `options`, and nothing in `spec.rs` copies it into a child — this
//! module is what carries it forward. Descending accumulates every node's
//! own persistent options into one running set keyed by name, so a nearer
//! declaration overwrites a farther one; the current node's own regular
//! `options` are still checked first, ahead of that set, so a name it
//! declares locally always wins over a persistent option of the same name
//! from further up.
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
//! still appear once something has started filling it. On a node that
//! declares subcommands it turns [`Walk::offers_options`] off as well: such
//! a node's own options belong in front of the subcommand rather than
//! behind it — `git -C sample status`, never `git status -C sample` — so a
//! word filling its arguments is past them too. `git`'s root is the case.
//! Its optional `alias` argument takes whatever word is not a subcommand,
//! and the global options left behind it are words Git itself refuses. A
//! node with no subcommand of its own has no such line to sit behind and
//! keeps offering its options after its arguments.
//!
//! **`loadSpec` re-rooting.** A node can itself be a pointer to another
//! spec rather than one of its own (`Subcommand::load_spec`); the
//! subcommand consumer follows it the moment it would otherwise descend,
//! before anything reads from the node it replaces. So can an argument:
//! `is_command` and `is_script` treat the word filling it as another
//! command's own name, `is_module` prepends its own prefix to that word
//! first, and `Arg::load_spec` is a straight pointer like a node's, read
//! before either and regardless of what the word says. `bin/console`, or
//! any path ending in it, is answered as the fixed global spec
//! `php/bin-console`; nothing else gets special treatment, so a local
//! script path is simply asked for and gets nothing back — which is what
//! "local" reduces to in this module. A successful re-root replaces the
//! current node with the loaded one, moves [`Walk::command_index`] to the
//! word that named it, and resets every other piece of state, since
//! nothing about the command so far belonged to this new spec. This is the
//! mechanism that makes `sudo git switch` walk `git`'s own spec from `git`
//! onward. `walk`'s `load_spec` parameter takes `impl Fn(&str) ->
//! Option<&'a Subcommand>` rather than an owned `Subcommand`, so a caller
//! holding a cache of already-loaded specs can simply hand out references
//! into it; this module never owns a node it did not receive as `root`.
//!
//! **The final word still only annotates.** Whatever argument is pending
//! when the loop stops — an option's own, or the current node's — becomes
//! [`Walk::current_arg`] and turns [`Walk::offers_args`] on, without being
//! consumed or advancing anything, and without ever triggering a re-root.
//! A forced pending argument also turns `offers_subcommands` and
//! `offers_options` off for that word, since nothing else could win it
//! either.

use crate::shellparse;
use crate::spec::{Arg, Opt, Repeatable, Separator, Subcommand};
use std::collections::HashMap;

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

/// The walk's own progress: where it is, and what it has already decided.
/// Kept as one value because a re-root replaces almost all of it at once.
struct State<'a> {
    node: &'a Subcommand,
    command_index: usize,
    end_of_options: bool,
    seen_non_option: bool,
    passed_options: Vec<&'a Opt>,
    option_arg: Option<OptionArg<'a>>,
    entered_subcommand_args: bool,
    subcommand_arg_index: usize,
    subcommand_arg_filled: bool,
    /// Every persistent option in scope, from `node` and every ancestor
    /// it took to reach it, keyed by name with the nearest declaration
    /// winning. `node`'s own regular `options` are not in here; they are
    /// checked first, separately, wherever this is read.
    ancestor_persistent: HashMap<&'a str, &'a Opt>,
}

impl<'a> State<'a> {
    fn new(root: &'a Subcommand) -> Self {
        let mut state = State {
            node: root,
            command_index: 0,
            end_of_options: false,
            seen_non_option: false,
            passed_options: Vec::new(),
            option_arg: None,
            entered_subcommand_args: false,
            subcommand_arg_index: 0,
            subcommand_arg_filled: false,
            ancestor_persistent: HashMap::new(),
        };
        state.absorb_persistent(root);
        state
    }

    fn absorb_persistent(&mut self, node: &'a Subcommand) {
        for (name, opt) in &node.persistent_options {
            self.ancestor_persistent.insert(name.as_str(), opt.as_ref());
        }
    }

    /// Descends into `child`, the subcommand the word at `index` named.
    fn descend(&mut self, child: &'a Subcommand, index: usize) {
        self.node = child;
        self.command_index = index;
        self.entered_subcommand_args = false;
        self.subcommand_arg_index = 0;
        self.subcommand_arg_filled = false;
        self.option_arg = None;
        self.absorb_persistent(child);
    }

    /// Starts over at `new_root`, as a `loadSpec` pointer or an
    /// `isCommand`-shaped argument's own value asks for.
    fn reroot(&mut self, new_root: &'a Subcommand, index: usize) {
        self.node = new_root;
        self.command_index = index;
        self.end_of_options = false;
        self.seen_non_option = false;
        self.passed_options.clear();
        self.option_arg = None;
        self.entered_subcommand_args = false;
        self.subcommand_arg_index = 0;
        self.subcommand_arg_filled = false;
        self.ancestor_persistent.clear();
        self.absorb_persistent(new_root);
    }

    /// Checks whether `arg`, about to be filled by `text`, re-roots the
    /// walk, and does so if it does. `index` is `text`'s own position,
    /// since a re-root always moves `command_index` there.
    fn maybe_reroot(
        &mut self,
        arg: &Arg,
        text: &str,
        index: usize,
        load_spec: &impl Fn(&str) -> Option<&'a Subcommand>,
    ) -> bool {
        let Some(name) = arg_reroot_name(arg, text) else {
            return false;
        };
        let Some(new_root) = load_spec(&name) else {
            return false;
        };
        self.reroot(new_root, index);
        true
    }

    /// An option by exact name, `node`'s own first and every ancestor's
    /// persistent option after.
    fn lookup_option(&self, name: &str) -> Option<&'a Opt> {
        if let Some(opt) = self.node.options.get(name) {
            return Some(opt.as_ref());
        }
        self.ancestor_persistent.get(name).copied()
    }

    fn active_variadic(&self) -> Option<&'a Arg> {
        if let Some((opt, arg_index, filled)) = self.option_arg {
            let arg = &opt.args[arg_index];
            return (filled && arg.is_variadic == Some(true)).then_some(arg);
        }
        if self.entered_subcommand_args
            && self.subcommand_arg_filled
            && self.subcommand_arg_index < self.node.args.len()
        {
            let arg = &self.node.args[self.subcommand_arg_index];
            if arg.is_variadic == Some(true) {
                return Some(arg);
            }
        }
        None
    }

    /// Whether the option consumer may still run at this point in the walk.
    fn can_consume_options(&self) -> bool {
        if self.end_of_options {
            return false;
        }
        if let Some(arg) = self.active_variadic()
            && !arg.options_can_break_variadic_arg.unwrap_or(true)
        {
            return false;
        }
        // A node that declares subcommands keeps its own options in front of
        // the subcommand rather than behind it: `git -C sample status`, never
        // `git status -C sample`. A word filling such a node's own arguments
        // is therefore past those options, the same way it is already past
        // the subcommand. `git`'s own root is the case this is for. Its
        // optional `alias` argument takes whatever word is not a subcommand,
        // and the global options left standing behind it are words Git itself
        // refuses. A node with no subcommand of its own has no such line to
        // sit behind and goes on offering its options after its arguments,
        // which is what `git add <file> -n` and `svn commit -m` both want.
        if self.entered_subcommand_args && !self.node.subcommands.is_empty() {
            return false;
        }
        let must_precede_arguments = self
            .node
            .parser_directives
            .as_ref()
            .is_some_and(|directives| directives.options_must_precede_arguments);
        !(must_precede_arguments && self.seen_non_option)
    }

    /// Tries every syntax the option consumer supports against one word, in
    /// the order a person would expect to win. The second element of a
    /// successful result is the pending-argument state the caller should
    /// adopt.
    fn consume_option(&self, text: &str) -> Option<(Vec<&'a Opt>, Option<OptionArg<'a>>)> {
        if let Some(opt) = self.lookup_option(text)
            && opt.requires_equals != Some(true)
            && is_available(opt, &self.passed_options)
        {
            return Some((vec![opt], start_pending(opt)));
        }

        if let Some(opt) = self.attached_option(text)
            && is_available(opt, &self.passed_options)
        {
            return Some((vec![opt], start_pending_from(opt, 1)));
        }

        let posix_noncompliant = self
            .node
            .parser_directives
            .as_ref()
            .is_some_and(|directives| directives.flags_are_posix_noncompliant);
        if !posix_noncompliant && text.starts_with('-') && !text.starts_with("--") {
            return self.short_chain(text);
        }

        None
    }

    /// A long option's value stuck to its name behind a separator, such as
    /// `--format=json`. Each option is tried against its own resolved
    /// separators: just the one `requires_separator` names when it names
    /// one, every separator `option_arg_separators` lists when it does
    /// not, and `=` alone when neither the option nor the spec names any.
    /// `node`'s own options are tried before a persistent option sharing a
    /// name with one of them, matching `lookup_option`.
    fn attached_option(&self, text: &str) -> Option<&'a Opt> {
        let mut candidates: Vec<&'a Opt> =
            self.node.options.values().map(|opt| opt.as_ref()).collect();
        for (name, opt) in &self.ancestor_persistent {
            if !self.node.options.contains_key(*name) {
                candidates.push(*opt);
            }
        }
        for opt in candidates {
            let Some(name) = opt.name.iter().find(|name| text.starts_with(name.as_str())) else {
                continue;
            };
            let rest = &text[name.len()..];
            if candidate_separators(self.node, opt)
                .iter()
                .any(|separator| rest.starts_with(separator.as_str()))
            {
                return Some(opt);
            }
        }
        None
    }

    /// A run of one-letter short options packed into one word, such as
    /// `-nvf`. Consumption reads left to right: the first letter whose own
    /// option takes an argument ends the chain there, with whatever
    /// follows read as that argument's stuck-on value — which is also how
    /// a single `-ovalue` word is read, as a chain that happens to be one
    /// letter long.
    fn short_chain(&self, text: &str) -> Option<(Vec<&'a Opt>, Option<OptionArg<'a>>)> {
        if text.len() <= 1 {
            return None; // a lone `-`: nothing to chain
        }
        let mut consumed = Vec::new();
        for (offset, letter) in text[1..].char_indices() {
            let opt = self.lookup_option(&format!("-{letter}"))?;
            if !is_available(opt, &self.passed_options) {
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
}

/// Walks `command`'s words against `root`, one word at a time.
///
/// `command.words[0]` is the command name that resolved to `root`; the walk
/// itself starts on `command.words[1]`. The caller loads `root` beforehand,
/// with [`crate::spec::load`] or otherwise, and answers `load_spec` from
/// wherever it keeps loaded specs, so this function does no I/O of its own.
pub fn walk<'a>(
    command: &shellparse::Command,
    root: &'a Subcommand,
    load_spec: impl Fn(&str) -> Option<&'a Subcommand>,
) -> Walk<'a> {
    let words = &command.words;
    let mut state = State::new(root);
    if let Some(target) = root.load_spec.first()
        && let Some(new_root) = load_spec(&target.name)
    {
        state.reroot(new_root, 0);
    }
    let mut search_term = String::new();

    if words.len() > 1 {
        let last_index = words.len() - 1;
        for (index, word) in words.iter().enumerate().take(last_index).skip(1) {
            let text = word.inner_text.as_str();

            // The option-argument consumer, forced: a pending argument that
            // still needs its first word and is not optional takes this
            // word no matter what it looks like — unless it re-roots first.
            if let Some((opt, arg_index, filled)) = state.option_arg
                && !filled
                && !opt.args[arg_index].is_optional.unwrap_or(false)
            {
                let arg = &opt.args[arg_index];
                if !state.maybe_reroot(arg, text, index, &load_spec) {
                    state.option_arg = advance_option_arg(opt, arg_index);
                }
                continue;
            }

            // The subcommand consumer. A child that is itself a `loadSpec`
            // pointer re-roots instead of being descended into.
            if !state.entered_subcommand_args
                && let Some(child) = state.node.subcommands.get(text)
            {
                let child = child.as_ref();
                let target = child
                    .load_spec
                    .first()
                    .and_then(|target| load_spec(&target.name));
                match target {
                    Some(new_root) => state.reroot(new_root, index),
                    None => state.descend(child, index),
                }
                continue;
            }

            // `--` and the option consumer.
            if state.can_consume_options() {
                match state.consume_option(text) {
                    Some((opts, pending)) => {
                        state.passed_options.extend(opts);
                        state.option_arg = pending;
                        continue;
                    }
                    None if text == "--" && !state.end_of_options => {
                        state.end_of_options = true;
                        continue;
                    }
                    None if text.starts_with('-') => break,
                    None => {}
                }
            }

            // The option-argument consumer, residual: a pending argument
            // that is optional, or already filled and merely continuing a
            // variadic run, takes whatever nothing else wanted.
            if let Some((opt, arg_index, _)) = state.option_arg {
                let arg = &opt.args[arg_index];
                if !state.maybe_reroot(arg, text, index, &load_spec) {
                    state.option_arg = advance_option_arg(opt, arg_index);
                }
                continue;
            }

            // The subcommand-argument consumer: the last resort, filling
            // the current node's own `args` in order.
            if state.subcommand_arg_index < state.node.args.len() {
                let node = state.node;
                let arg_index = state.subcommand_arg_index;
                let arg = &node.args[arg_index];
                if !state.maybe_reroot(arg, text, index, &load_spec) {
                    if arg.is_variadic == Some(true) {
                        state.subcommand_arg_filled = true;
                    } else {
                        state.subcommand_arg_index += 1;
                        state.subcommand_arg_filled = false;
                    }
                    state.entered_subcommand_args = true;
                    state.seen_non_option = true;
                }
                continue;
            }

            state.seen_non_option = true;
        }
        search_term = words[last_index].inner_text.clone();
    }

    let forced = matches!(
        state.option_arg,
        Some((opt, arg_index, filled))
            if !filled && !opt.args[arg_index].is_optional.unwrap_or(false)
    );
    let current_arg = if let Some((opt, arg_index, _)) = state.option_arg {
        Some(opt.args[arg_index].clone())
    } else if state.subcommand_arg_index < state.node.args.len() {
        Some(state.node.args[state.subcommand_arg_index].clone())
    } else {
        None
    };

    Walk {
        node: state.node,
        offers_args: current_arg.is_some(),
        current_arg,
        offers_subcommands: !state.entered_subcommand_args && !forced,
        offers_options: !forced && state.can_consume_options(),
        command_index: state.command_index,
        end_of_options: state.end_of_options,
        search_term,
        passed_options: state.passed_options,
    }
}

/// The name to ask the loader for when `text` fills `arg`, if `arg` is one
/// of the kinds that re-roots the walk: a fixed `loadSpec` pointer — read
/// regardless of what `text` says, since the corpus's one example is a
/// straight redirect — an `isCommand` or `isScript` argument naming the
/// word itself, or an `isModule` argument prepending its own prefix to it.
fn arg_reroot_name(arg: &Arg, text: &str) -> Option<String> {
    if let Some(target) = arg.load_spec.first() {
        return Some(target.name.clone());
    }
    if arg.is_command == Some(true) || arg.is_script == Some(true) {
        return Some(command_lookup_name(text).to_string());
    }
    if let Some(prefix) = &arg.is_module {
        return Some(format!("{prefix}{text}"));
    }
    None
}

/// The global spec name a token that names a command or a script resolves
/// to. `bin/console`, or any path ending in it, is a Symfony-style entry
/// point the corpus answers under one fixed name; everything else is asked
/// for exactly as typed. A path with no compiled-in answer — a `/`, `./` or
/// `~/` local script, almost always — simply gets `None` back from the
/// loader, which is what "local" reduces to in this module.
fn command_lookup_name(text: &str) -> &str {
    if text == "bin/console" || text.ends_with("/bin/console") {
        "php/bin-console"
    } else {
        text
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{self, LoadSpec, ParserDirectives, Repeatable};
    use std::rc::Rc;

    fn command(line: &str) -> shellparse::Command {
        shellparse::parse(line).commands.into_iter().next().unwrap()
    }

    #[test]
    fn a_trailing_space_offers_the_root_subcommands() {
        let git = spec::load("git", &[]).unwrap();
        let walk = walk(&command("git "), &git, |_| None);
        assert!(std::ptr::eq(walk.node, &git));
        assert_eq!(walk.search_term, "");
        assert!(walk.offers_subcommands);
    }

    #[test]
    fn a_partial_word_is_the_search_term_and_does_not_descend() {
        let git = spec::load("git", &[]).unwrap();
        let walk = walk(&command("git swi"), &git, |_| None);
        assert!(std::ptr::eq(walk.node, &git));
        assert_eq!(walk.search_term, "swi");
        assert!(walk.offers_subcommands);
    }

    #[test]
    fn a_full_subcommand_and_trailing_space_descends() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git switch "), &git, |_| None);
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
        let walk = walk(&command("git switch ma"), &git, |_| None);
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
        let walk = walk(&command("docker container ls "), &docker, |_| None);
        assert!(std::ptr::eq(walk.node, ls.as_ref()));
        assert_eq!(walk.search_term, "");
        assert_eq!(walk.command_index, 2);
    }

    #[test]
    fn a_word_that_names_no_subcommand_stops_the_walk_where_it_stopped() {
        let git = spec::load("git", &[]).unwrap();
        let walk = walk(&command("git nosuchcommand extra "), &git, |_| None);
        assert!(std::ptr::eq(walk.node, &git));
        assert_eq!(walk.command_index, 0);
        assert_eq!(walk.search_term, "");
    }

    #[test]
    fn a_bare_long_option_is_taken_even_though_its_value_is_a_separate_word() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let create = switch.options.get("--create").unwrap();
        let walk = walk(&command("git switch --create newbranch "), &git, |_| None);
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
        let walk = walk(&command("git add -nvf "), &git, |_| None);
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
        let walk = walk(&command("git switch -cnewbranch "), &git, |_| None);
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], create.as_ref()));
    }

    #[test]
    fn a_double_dash_mid_line_ends_options_without_blocking_a_descend() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git -- switch "), &git, |_| None);
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
        let walk = walk(&command("git -- -- switch "), &git, |_| None);
        assert!(walk.end_of_options);
        assert!(std::ptr::eq(walk.node, &git));
        assert!(!walk.offers_subcommands);
    }

    /// The other half of that rule. `git`'s root declares subcommands, so a
    /// word spent on its `alias` argument is past its own options as well,
    /// and `git sample --bare` is a line Git refuses.
    #[test]
    fn a_word_that_fills_the_roots_own_argument_closes_off_its_options_too() {
        let git = spec::load("git", &[]).unwrap();
        assert!(!git.subcommands.is_empty());
        let walk = walk(&command("git sample "), &git, |_| None);
        assert!(std::ptr::eq(walk.node, &git));
        assert!(!walk.offers_subcommands);
        assert!(!walk.offers_options);
    }

    /// The same word on a node with no subcommand of its own leaves the
    /// options where they were. `git add <file> -n` is a line Git takes.
    #[test]
    fn a_filled_argument_keeps_the_options_of_a_node_with_no_subcommands() {
        let git = spec::load("git", &[]).unwrap();
        let add = git.subcommands.get("add").unwrap();
        assert!(add.subcommands.is_empty());
        let walk = walk(&command("git add file1 "), &git, |_| None);
        assert!(std::ptr::eq(walk.node, add.as_ref()));
        assert!(walk.offers_options);
    }

    #[test]
    fn a_double_dash_as_the_final_word_is_only_the_search_term() {
        let git = spec::load("git", &[]).unwrap();
        let walk = walk(&command("git --"), &git, |_| None);
        assert!(std::ptr::eq(walk.node, &git));
        assert_eq!(walk.search_term, "--");
        assert!(!walk.end_of_options);
        assert!(walk.passed_options.is_empty());
    }

    #[test]
    fn options_must_precede_arguments_refuses_an_option_once_a_plain_word_has_gone_by() {
        let fold = spec::load("fold", &[]).unwrap();
        let walk = walk(&command("fold somefile -b "), &fold, |_| None);
        assert!(walk.passed_options.is_empty());
    }

    #[test]
    fn options_still_consume_fine_when_no_plain_word_has_gone_by() {
        let fold = spec::load("fold", &[]).unwrap();
        let b = fold.options.get("-b").unwrap();
        let s = fold.options.get("-s").unwrap();
        let walk = walk(&command("fold -b -s "), &fold, |_| None);
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], b.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[1], s.as_ref()));
    }

    #[test]
    fn each_options_own_explicit_separator_is_honored() {
        let esbuild = spec::load("esbuild", &[]).unwrap();
        let loader = esbuild.options.get("--loader").unwrap();
        let format = esbuild.options.get("--format").unwrap();
        let walk = walk(
            &command("esbuild --loader:js --format=esm "),
            &esbuild,
            |_| None,
        );
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], loader.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[1], format.as_ref()));
    }

    #[test]
    fn requires_equals_refuses_the_bare_form() {
        let mosh = spec::load("mosh", &[]).unwrap();
        let walk = walk(&command("mosh --predict --family=inet "), &mosh, |_| None);
        assert!(walk.passed_options.is_empty());
        assert!(std::ptr::eq(walk.node, &mosh));
        assert_eq!(walk.command_index, 0);
    }

    #[test]
    fn requires_equals_accepts_the_attached_form() {
        let mosh = spec::load("mosh", &[]).unwrap();
        let predict = mosh.options.get("--predict").unwrap();
        let walk = walk(
            &command("mosh --predict=always --family=inet "),
            &mosh,
            |_| None,
        );
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], predict.as_ref()));
    }

    #[test]
    fn requires_separator_default_resolves_to_the_spec_wide_fallback() {
        let ua = spec::load("ua", &[]).unwrap();
        let status = ua.subcommands.get("status").unwrap();
        let format = status.options.get("--format").unwrap();
        let walk = walk(&command("ua status --format=json "), &ua, |_| None);
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], format.as_ref()));
    }

    #[test]
    fn requires_separator_explicit_names_its_own_separator() {
        let eza = spec::load("eza", &[]).unwrap();
        let color_scale = eza.options.get("--color-scale").unwrap();
        let walk = walk(
            &command("eza --color-scale=all --color=auto "),
            &eza,
            |_| None,
        );
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
            |_| None,
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

        let via_colon = walk(&command("probe --level:5 "), &node, |_| None);
        assert_eq!(via_colon.passed_options.len(), 1);

        let via_equals = walk(&command("probe --level=5 "), &node, |_| None);
        assert!(via_equals.passed_options.is_empty());
    }

    #[test]
    fn flags_are_posix_noncompliant_refuses_to_decompose_a_chain() {
        let kubectx = spec::load("kubectx", &[]).unwrap();
        let walk = walk(&command("kubectx -hc "), &kubectx, |_| None);
        assert!(walk.passed_options.is_empty());
        assert!(std::ptr::eq(walk.node, &kubectx));
    }

    #[test]
    fn a_final_word_that_looks_like_an_option_only_annotates() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git switch --c"), &git, |_| None);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
        assert_eq!(walk.search_term, "--c");
        assert!(walk.offers_options);
        assert!(walk.passed_options.is_empty());
    }

    #[test]
    fn a_final_word_that_looks_like_an_option_does_not_offer_options_once_they_are_off() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let walk = walk(&command("git switch -- --c"), &git, |_| None);
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
        let walk = walk(&command("git switch --create -weirdname "), &git, |_| None);
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
        let walk = walk(&command("git switch "), &git, |_| None);
        assert!(std::ptr::eq(walk.node, switch.as_ref()));
    }

    #[test]
    fn a_variadic_subcommand_argument_takes_several_words() {
        let git = spec::load("git", &[]).unwrap();
        let add = git.subcommands.get("add").unwrap();
        let walk = walk(&command("git add file1 file2 file3 "), &git, |_| None);
        assert!(std::ptr::eq(walk.node, add.as_ref()));
        assert_eq!(walk.current_arg.unwrap().name, add.args[0].name);
        assert!(walk.offers_args);
    }

    #[test]
    fn option_repeatable_once_refuses_a_second_occurrence() {
        let git = spec::load("git", &[]).unwrap();
        let switch = git.subcommands.get("switch").unwrap();
        let detach = switch.options.get("-d").unwrap();
        let walk = walk(&command("git switch -d -d "), &git, |_| None);
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], detach.as_ref()));
    }

    #[test]
    fn option_repeatable_times_caps_at_its_count() {
        let git = spec::load("git", &[]).unwrap();
        let branch = git.subcommands.get("branch").unwrap();
        let verbose = branch.options.get("-v").unwrap();
        let walk = walk(&command("git branch -v -v -v "), &git, |_| None);
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
            |_| None,
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
        let walk = walk(&command("git add file1 -n file2 "), &git, |_| None);
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
        let walk = walk(&command("git diff -- file1 --staged file2 "), &git, |_| {
            None
        });
        // Only `--` itself was consumed as an option; `--staged` was read as
        // another path instead of interrupting the variadic one.
        assert_eq!(walk.passed_options.len(), 1);
        assert!(std::ptr::eq(walk.passed_options[0], dashdash.as_ref()));
        assert!(std::ptr::eq(walk.node, diff.as_ref()));
        assert!(walk.offers_args);
    }

    /// `chezmoi`'s root declares 26 persistent options including
    /// `--dry-run`/`-n`; `state dump`, two levels down, has to reach it.
    /// `dump` also declares `--format`/`-f` as persistent of its own, so
    /// this is the chain with two contributors rather than one: the root's
    /// and the current node's own.
    #[test]
    fn persistent_options_accumulate_from_every_ancestor_two_levels_down() {
        let chezmoi = spec::load("chezmoi", &[]).unwrap();
        let dry_run = chezmoi.persistent_options.get("--dry-run").unwrap();
        let state = chezmoi.subcommands.get("state").unwrap();
        let dump = state.subcommands.get("dump").unwrap();
        let own_format = dump.persistent_options.get("--format").unwrap();
        let walk = walk(
            &command("chezmoi state dump --dry-run --format=yaml "),
            &chezmoi,
            |_| None,
        );
        assert!(std::ptr::eq(walk.node, dump.as_ref()));
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], dry_run.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[1], own_format.as_ref()));
    }

    /// `bws`'s root declares `--output`, `--profile` and `--config-file`
    /// (among others) as persistent; its `config` subcommand redeclares
    /// `--profile` and `--config-file` as its own, ordinary options.
    /// `--output` is not redeclared, so it still comes from the root;
    /// `--profile` resolves to `config`'s own, nearer declaration.
    #[test]
    fn a_childs_own_option_wins_over_an_ancestors_persistent_option_of_the_same_name() {
        let bws = spec::load("bws", &[]).unwrap();
        let output = bws.persistent_options.get("--output").unwrap();
        let config = bws.subcommands.get("config").unwrap();
        let own_profile = config.options.get("--profile").unwrap();
        let walk = walk(
            &command("bws config --output json --profile default "),
            &bws,
            |_| None,
        );
        assert!(std::ptr::eq(walk.node, config.as_ref()));
        assert_eq!(walk.passed_options.len(), 2);
        assert!(std::ptr::eq(walk.passed_options[0], output.as_ref()));
        assert!(std::ptr::eq(walk.passed_options[1], own_profile.as_ref()));
    }

    /// `aws`'s own `account` subcommand is a `loadSpec` pointer to
    /// `aws/account` rather than a node of its own.
    #[test]
    fn a_subcommand_load_spec_reroots_from_the_token_that_named_it() {
        let aws = spec::load("aws", &[]).unwrap();
        let account = spec::load("aws/account", &[]).unwrap();
        let walk = walk(&command("aws account "), &aws, |name| {
            (name == "aws/account").then_some(&account)
        });
        assert!(std::ptr::eq(walk.node, &account));
        assert_eq!(walk.command_index, 1);
        assert!(walk.offers_subcommands); // `account`'s own subcommands, not `aws`'s
    }

    /// `sudo`'s own root argument is `isCommand`, with no `loadSpec` and no
    /// subcommand in sight — the word itself names the command to re-root
    /// to, exactly as `sudo git switch` needs.
    #[test]
    fn is_command_reroots_from_the_token_that_named_it() {
        let sudo = spec::load("sudo", &[]).unwrap();
        let git = spec::load("git", &[]).unwrap();
        let load = |name: &str| (name == "git").then_some(&git);

        let stops_at_the_reroot = walk(&command("sudo git "), &sudo, load);
        assert!(std::ptr::eq(stops_at_the_reroot.node, &git));
        assert_eq!(stops_at_the_reroot.command_index, 1);

        let switch = git.subcommands.get("switch").unwrap();
        let continues_walking_the_new_spec = walk(&command("sudo git switch "), &sudo, load);
        assert!(std::ptr::eq(
            continues_walking_the_new_spec.node,
            switch.as_ref()
        ));
        assert_eq!(continues_walking_the_new_spec.command_index, 2);
    }

    #[test]
    fn is_command_leaves_the_walk_alone_when_the_loader_declines() {
        let sudo = spec::load("sudo", &[]).unwrap();
        let walk = walk(&command("sudo made-up-command "), &sudo, |_| None);
        assert!(std::ptr::eq(walk.node, &sudo));
    }

    /// `bin/console` names no command of its own; it is the corpus's fixed
    /// alias for the Symfony-style entry point `php/bin-console`, and that
    /// spec is itself committed.
    #[test]
    fn is_command_maps_bin_console_to_its_fixed_global_spec() {
        let sudo = spec::load("sudo", &[]).unwrap();
        let bin_console = spec::load("php/bin-console", &[]).unwrap();
        let walk = walk(&command("sudo bin/console "), &sudo, |name| {
            (name == "php/bin-console").then_some(&bin_console)
        });
        assert!(std::ptr::eq(walk.node, &bin_console));
    }

    /// `osascript`'s own root argument is `isScript`: its value is a file
    /// path, never a name any compiled-in spec answers to, so the loader
    /// is asked and — realistically, always — declines.
    #[test]
    fn is_script_asks_the_loader_and_typically_gets_nothing_back() {
        let osascript = spec::load("osascript", &[]).unwrap();
        let walk = walk(&command("osascript myscript.scpt "), &osascript, |_| None);
        assert!(std::ptr::eq(walk.node, &osascript));
    }

    /// `python`'s `-m` option takes one argument whose `isModule` is
    /// `"python/"`; `python/http.server` is itself a committed spec, so
    /// `python -m http.server` re-roots to it for real.
    #[test]
    fn is_module_prepends_its_prefix_to_the_token() {
        let python = spec::load("python", &[]).unwrap();
        let http_server = spec::load("python/http.server", &[]).unwrap();
        let walk = walk(&command("python -m http.server "), &python, |name| {
            (name == "python/http.server").then_some(&http_server)
        });
        assert!(std::ptr::eq(walk.node, &http_server));
        assert_eq!(walk.command_index, 2);
    }

    /// No committed argument carries its own `loadSpec` in a shape reachable
    /// without a long, unrelated chain of subcommands to get there, so this
    /// one argument is built by hand; the node it lives on, the word that
    /// fills it and the spec it re-roots to are otherwise unremarkable.
    #[test]
    fn an_arg_level_load_spec_reroots_regardless_of_the_word_it_names() {
        let docker = spec::load("docker", &[]).unwrap();
        let mut node = bare_node(HashMap::new(), ParserDirectives::default());
        node.args = vec![Arg {
            load_spec: vec![LoadSpec {
                name: "docker".to_string(),
                kind: "global".to_string(),
            }],
            ..Arg::default()
        }];
        let walk = walk(&command("probe anything "), &node, |name| {
            (name == "docker").then_some(&docker)
        });
        assert!(std::ptr::eq(walk.node, &docker));
        assert_eq!(walk.command_index, 1);
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
