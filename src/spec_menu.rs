//! Completion from a command's own specification.
//!
//! Where Git and `cd` each read their own syntax, this reads whatever
//! [`crate::spec`] has for the command name in front of the cursor. `App`
//! reaches for it only once neither of the other two has claimed the line.

use crate::argwalk::{self, Walk};
use crate::candidates::{
    Candidate, DEFAULT_PRIORITY, FOLDER, Kind, Query, Scan, UsedAfter, priority_of, rank,
    resolved_in, split,
};
use crate::fuzzy;
use crate::histfile;
use crate::history::History;
use crate::native;
use crate::shellparse::{self, Command};
use crate::spec::{self, Generator, Opt, Subcommand};
use serde_json::Value;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

/// How many times [`Completions::complete`] retries a walk that asked for a
/// spec it had not loaded yet. `sudo git switch` re-roots once, from `sudo`
/// to `git`; a re-rooted node can itself point at a third spec, so one retry
/// is not always enough. Nothing in the corpus chains anywhere near this
/// deep — it exists so a cyclic `loadSpec` pointer degrades to no rows
/// rather than spinning forever.
const MAX_REROOT_PASSES: u32 = 8;

/// A command this provider answered `left` for: the final word it would
/// replace, and the parsed command that word sits in. `App::refresh` reads
/// `word` the way it already reads a Git `Target`'s, and hands the whole
/// thing to [`Completions::complete`] rather than asking `App` to parse the
/// line a second time.
pub(crate) struct Target {
    pub word: Query,
    command: Command,
}

/// Read a command from `left`, once neither Git's own menu nor `cd`'s claims
/// it for itself. `tail` is what sits to the right of the cursor; a tail that
/// does not start on a space is the middle of a word the cursor has not
/// finished, the same case `App::arg` refuses for Git and `cd`.
///
/// `git` and `cd` are refused by name here rather than left to fall through:
/// their own menus sit above this one in `App::refresh`, but their own
/// parsers are narrower than a spec's own walk and decline lines this would
/// otherwise happily answer, such as `git sample ` — a finished subcommand
/// with a trailing space, which `crate::git::parse` no longer reads as a Git
/// line at all. Answering there anyway would let this provider quietly take
/// over a line the other two are still meant to own; this is what keeps this
/// provider to only what the first two declined.
///
/// The refusal reads the raw word, not what an alias expands it to.
/// `crate::git::parse` and `cd`'s own reader each match a literal `git` or
/// `cd` at the start of the line; neither ever claims a line that starts
/// with an alias for one. Refusing on the expanded name would therefore
/// hold this provider off a line nobody else is going to answer, and
/// `alias g=git` would open on nothing at all.
pub(crate) fn parse(left: &str, tail: &str, aliases: &HashMap<String, String>) -> Option<Target> {
    if !tail.is_empty() && !tail.starts_with(char::is_whitespace) {
        return None;
    }
    let command = command_at_cursor(left, aliases)?;
    // One word is the command name, still being typed. That is the shell's
    // own completion to offer, not an argument to walk.
    if command.words.len() < 2 {
        return None;
    }
    let raw = shellparse::command_at(left, left.len());
    let raw_name = raw.as_ref().and_then(|cmd| cmd.words.first());
    // Only what Git's own menu and `cd`'s declined, never what they claim.
    if raw_name.is_some_and(|w| w.inner_text == "git" || w.inner_text == "cd") {
        return None;
    }
    let word = command.words.last()?;
    let query = Query {
        start: word.start,
        arg: left[word.start..word.end].to_string(),
    };
    Some(Target {
        word: query,
        command,
    })
}

/// The label a row falls back to when the spec gives it no description.
/// Git's own command rows already fall back to `"command"`; the other two
/// are this module's own choice, made for the same reason.
const SUBCOMMAND_LABEL: &str = "command";
const OPTION_LABEL: &str = "option";
const SUGGESTION_LABEL: &str = "value";
const FILE_LABEL: &str = "file";

/// Loads and walks specs for one menu. `App` keeps this behind the same
/// field for the life of a menu that `crate::git::Completions` is kept
/// behind, so a spec is read from disk once regardless of how many keys
/// follow — a re-rooted one included, since a re-root can ask for a second
/// or third name in the same menu and every one of them lands in here too.
#[derive(Default)]
pub(crate) struct Completions {
    /// Every spec this menu has asked for, by name. `None` is a cached miss:
    /// `SpecError::Missing` or a corrupt build, either of which stays a miss
    /// for the rest of the menu without asking `spec::load_configured`
    /// again.
    specs: HashMap<String, Option<Rc<Subcommand>>>,
    /// How often `$HISTFILE` says the line's own first word was followed by
    /// a given name. `App` sets this once; every other field here is this
    /// menu's own cache and this one is data handed in from outside it.
    pub(crate) cmd_history: histfile::Counts,
}

impl Completions {
    /// The rows `target` offers. `App::refresh` calls this only once `parse`
    /// has already found a command for the current line. `cwd`, `history`
    /// and `scan` are the same three a `cd` menu answers from; a `filepaths`
    /// or `folders` template reads the filesystem and the directory history
    /// the identical way `cd` itself does.
    pub(crate) fn complete(
        &mut self,
        target: &Target,
        cwd: &Path,
        history: &History,
        scan: &mut Scan,
    ) -> Vec<Candidate> {
        let Some(root) = self.spec_for(&target.command.words[0].inner_text) else {
            return Vec::new();
        };
        self.walk_resolving(target, &root, cwd, history, scan)
    }

    /// Walks `target.command` against `root`, loading whatever a re-root
    /// asks for that this menu has not cached yet and trying again. A node
    /// that itself points at a third spec only comes into view once the
    /// second one is loaded, so one retry is not assumed to be enough; a
    /// pass that asks for nothing new is what ends the loop, rather than a
    /// fixed count of them.
    ///
    /// The loader handed to `argwalk::walk` only ever reads `self.specs` and
    /// records a miss in `asked`, a plain local `Vec` behind a `RefCell` of
    /// its own; nothing here needs `self` mutably until the walk and its
    /// borrow of `self.specs` are both done with for this pass, so loading
    /// the newly asked-for names after it stays an ordinary `&mut self` call
    /// rather than something that needs interior mutability of its own.
    fn walk_resolving(
        &mut self,
        target: &Target,
        root: &Rc<Subcommand>,
        cwd: &Path,
        history: &History,
        scan: &mut Scan,
    ) -> Vec<Candidate> {
        for _ in 0..MAX_REROOT_PASSES {
            let asked: RefCell<Vec<String>> = RefCell::new(Vec::new());
            let walk = argwalk::walk(&target.command, root, |name| match self.specs.get(name) {
                Some(found) => found.as_deref(),
                None => {
                    asked.borrow_mut().push(name.to_string());
                    None
                }
            });
            let new_names: Vec<String> = asked
                .into_inner()
                .into_iter()
                .filter(|name| !self.specs.contains_key(name))
                .collect();
            if new_names.is_empty() {
                // `help`'s siblings live one word back from the current
                // node; that walk asks for nothing this one has not already
                // loaded, so it only runs when a `help` template is actually
                // in play.
                let wants_help = walk.current_arg.as_ref().is_some_and(|arg| {
                    arg.generators
                        .iter()
                        .any(|g| g.template.iter().any(|t| t == "help"))
                });
                let enclosing = wants_help
                    .then(|| enclosing_node(&self.specs, &target.command, root, walk.command_index))
                    .flatten();
                let mut rows = build_rows(&walk, enclosing, cwd, history, scan);
                // Two words means the one being replaced is the command's
                // own second, which is the only word `cmd_history` counted.
                // A deeper subcommand, an option's value and anything past
                // an option already typed all sit further along; see `rank`.
                let used_after = (target.command.words.len() == 2).then(|| UsedAfter {
                    command: target.command.words[0].inner_text.as_str(),
                    counts: &self.cmd_history,
                });
                rank(&mut rows, walk.search_term.as_str(), used_after);
                return rows;
            }
            for name in new_names {
                self.load(&name);
            }
        }
        Vec::new()
    }

    fn spec_for(&mut self, name: &str) -> Option<Rc<Subcommand>> {
        if !self.specs.contains_key(name) {
            self.load(name);
        }
        self.specs.get(name).and_then(|found| found.clone())
    }

    fn load(&mut self, name: &str) {
        let loaded = spec::load_configured(name).ok().map(Rc::new);
        self.specs.insert(name.to_string(), loaded);
    }
}

/// The command holding the cursor, with `aliases` already resolved on its
/// name. [`shellparse::command_at`] answers the same question over a plain
/// [`shellparse::parse`]; an alias reaches a spec under the name it expands
/// to, so this reads [`shellparse::parse_with_aliases`] instead and finds
/// the command the same way `command_at` does.
fn command_at_cursor(left: &str, aliases: &HashMap<String, String>) -> Option<Command> {
    let cursor = left.len();
    shellparse::parse_with_aliases(left, aliases)
        .commands
        .into_iter()
        .find(|cmd| cmd.start <= cursor && cursor <= cmd.end)
}

/// Every value in `map` once, by the object it names rather than by the name
/// that reaches it. An alias and the name it stands for share one `Rc` and a
/// map keyed by name alone would otherwise show the same subcommand or
/// option once per name it answers to.
fn unique_targets<T>(map: &HashMap<String, Rc<T>>) -> Vec<&Rc<T>> {
    let mut seen = HashSet::new();
    map.values()
        .filter(|rc| seen.insert(Rc::as_ptr(rc)))
        .collect()
}

fn label(description: &Option<String>, fallback: &'static str) -> Cow<'static, str> {
    match description {
        Some(text) => Cow::Owned(text.clone()),
        None => Cow::Borrowed(fallback),
    }
}

fn row(
    term: &str,
    name: &str,
    label: Cow<'static, str>,
    hint: Vec<String>,
    kind: Kind,
    priority: u8,
) -> Option<Candidate> {
    Some(Candidate {
        display: name.to_string(),
        insert: name.to_string(),
        label,
        hint,
        kind,
        score: fuzzy::score(term, name)?,
        priority,
    })
}

/// The rows one walk offers, unranked. `enclosing` is the node a `help`
/// template's rows come from; every other caller passes `None`.
/// `walk_resolving` ranks what this returns against [`Walk::search_term`].
fn build_rows(
    walk: &Walk,
    enclosing: Option<&Subcommand>,
    cwd: &Path,
    history: &History,
    scan: &mut Scan,
) -> Vec<Candidate> {
    let term = walk.search_term.as_str();
    let mut rows = Vec::new();

    if walk.offers_subcommands {
        for sub in unique_targets(&walk.node.subcommands) {
            let Some(name) = sub.name.first() else {
                continue;
            };
            rows.extend(row(
                term,
                name,
                label(&sub.description, SUBCOMMAND_LABEL),
                spec::arg_hints(&sub.args),
                Kind::Command,
                priority_of(sub.priority),
            ));
        }
    }

    if walk.offers_options {
        for opt in unique_targets(&walk.node.options) {
            if already_passed(&walk.passed_options, opt) {
                continue;
            }
            let Some(name) = opt.name.first() else {
                continue;
            };
            // A space between the name and the hint is what says the
            // value is a word of its own. An option that requires a
            // separator takes `--name=value` instead, and the row cannot
            // say so until the separator is part of what it inserts, which
            // `plans/phase-3-ranking-insertion.md` is where that lands. It
            // shows no arguments until then rather than the wrong shape.
            let hint = match opt.requires_separator {
                Some(_) => Vec::new(),
                None => spec::arg_hints(&opt.args),
            };
            rows.extend(row(
                term,
                name,
                label(&opt.description, OPTION_LABEL),
                hint,
                Kind::Option,
                priority_of(opt.priority),
            ));
        }
    }

    if walk.offers_args
        && let Some(arg) = &walk.current_arg
    {
        for suggestion in &arg.suggestions {
            let Some(name) = suggestion.name.first() else {
                continue;
            };
            // A suggestion is a fixed value rather than a file Git would
            // recognise. `Kind::Path` is the nearest existing kind: it sits
            // in the same flat, non-directory group as `Command` and
            // `Option`, and its quoting is a plain shell word rather than a
            // Git pathspec.
            rows.extend(row(
                term,
                name,
                label(&suggestion.description, SUGGESTION_LABEL),
                Vec::new(),
                Kind::Path,
                priority_of(suggestion.priority),
            ));
        }
        for generator in &arg.generators {
            rows.extend(generator_rows(
                generator, term, cwd, history, scan, enclosing,
            ));
        }
        // An argument the conversion could not keep the code for. `native`
        // has a reader for two of them and nothing at all for the rest,
        // which keep offering no rows.
        if arg.dynamic
            && let Some(owner) = walk.node.name.first()
            && let Some(name) = arg.name.first()
        {
            rows.extend(native::rows(owner, name, term, cwd));
        }
    }

    rows
}

/// The rows one generator on the current argument offers, by its template.
/// `q-inventory-npm.md` §2 and `plans/phase-2-spec-runtime.md` §3 are the
/// whole of what a template can name; a generator naming none of them, or
/// naming `history` (phase 5's own reader), offers nothing here.
fn generator_rows(
    generator: &Generator,
    term: &str,
    cwd: &Path,
    history: &History,
    scan: &mut Scan,
    enclosing: Option<&Subcommand>,
) -> Vec<Candidate> {
    let names = |wanted: &str| generator.template.iter().any(|t| t == wanted);
    if names("filepaths") {
        path_rows(generator, term, cwd, history, scan, false)
    } else if names("folders") {
        path_rows(generator, term, cwd, history, scan, true)
    } else if names("help") {
        help_rows(term, enclosing)
    } else {
        Vec::new()
    }
}

/// Rows for an argument whose generator names the `filepaths` or `folders`
/// template. Both read the directory the argument names from the
/// filesystem through the same [`Scan`] cache `cd` uses, one walk per menu
/// rather than one per key. `folders_only` is what `folders` sets on itself:
/// npm §2 has it as `filepaths` with `showFolders: "only"`.
///
/// npm §2 lists `extensions`, `equals`, `matches`, `filterFolders`,
/// `rootDirectory`, `editFileSuggestions` and `editFolderSuggestions`
/// alongside `showFolders`. None of the seven appears once in the 1481
/// committed specs (measured across every generator and every argument), so
/// none of them is read here; `showFolders` is the one option this reads,
/// because `folders` is defined as `filepaths` with it forced to `"only"`
/// and the mechanism has to exist for that alone. A spec added later that
/// sets one of the other seven is silently not honoured — this sentence is
/// what a person chasing that down should find.
fn path_rows(
    generator: &Generator,
    raw_term: &str,
    cwd: &Path,
    history: &History,
    scan: &mut Scan,
    folders_only: bool,
) -> Vec<Candidate> {
    // `apply_path_defaults` in `spec.rs` sets this to `/` for both templates
    // unless the corpus already carried something else for it. A generator
    // that reaches here with anything else is one this reader has no split
    // rule for, the same way a `dyn` argument has no generator at all.
    if generator.get_query_term.as_deref() != Some("/") {
        return Vec::new();
    }
    let (prefix, term) = split(raw_term);
    let dir = if prefix.is_empty() {
        cwd.to_path_buf()
    } else {
        resolved_in(prefix, cwd)
    };
    let show_folders = if folders_only {
        "only"
    } else {
        generator
            .extra
            .get("showFolders")
            .and_then(Value::as_str)
            .unwrap_or("always")
    };

    let mut out = Vec::new();
    for (name, is_dir) in scan.entries(&dir, term.starts_with('.')) {
        let is_dir = *is_dir;
        match (is_dir, show_folders) {
            (true, "never") | (false, "only") => continue,
            _ => {}
        }
        let Some(mut score) = fuzzy::score(term, name) else {
            continue;
        };
        // File rows never touch history; `cd`'s own weighting is a folder
        // idea and this keeps it one.
        if is_dir {
            score += history_bonus(history, &dir.join(name));
        }
        let sep = if is_dir { "/" } else { "" };
        out.push(Candidate {
            display: format!("{name}{sep}"),
            insert: format!("{prefix}{name}{sep}"),
            label: if is_dir {
                Cow::Borrowed(FOLDER)
            } else {
                Cow::Borrowed(FILE_LABEL)
            },
            hint: Vec::new(),
            kind: Kind::Path,
            score,
            priority: DEFAULT_PRIORITY,
        });
    }
    out
}

/// The score bonus a visited folder earns, folded into the fuzzy score
/// rather than kept as a second sort key: every row this argument offers
/// shares one ranking pass with the rest of the spec provider's rows, and a
/// second key would fork that pass in two. `cd`'s own sort puts history
/// ahead of the fuzzy score outright; the scale here does the same in
/// practice, since one recent visit already outweighs any difference two
/// fuzzy scores would otherwise carry.
fn history_bonus(history: &History, target: &Path) -> i32 {
    (history.weight(target) * 1000.0) as i32
}

/// `help`'s own rows: the subcommands of the node one word back from the
/// current one, so `fnm help <x>` offers `fnm`'s own subcommands rather than
/// `help`'s own, which has none. `None` when the walk never left the
/// command name or found nothing to reach.
fn help_rows(term: &str, enclosing: Option<&Subcommand>) -> Vec<Candidate> {
    let Some(node) = enclosing else {
        return Vec::new();
    };
    unique_targets(&node.subcommands)
        .into_iter()
        .filter_map(|sub| {
            let name = sub.name.first()?;
            row(
                term,
                name,
                label(&sub.description, SUBCOMMAND_LABEL),
                // The row fills `help`'s own argument with this name and
                // stops there. What the sibling itself takes is never
                // reached, so a hint here would promise a word the menu
                // will not offer.
                Vec::new(),
                Kind::Command,
                priority_of(sub.priority),
            )
        })
        .collect()
}

/// The node one word back from `node_index`: `argwalk::walk` run again on
/// only the words up to and including it, so the walk stops short of
/// descending into it rather than reaching it. `help`'s siblings are that
/// node's own subcommands. Every spec this could ask for was already loaded
/// by the walk `node_index` itself came from, since a prefix of the same
/// words can only re-tread reroots that walk already resolved, so the
/// loader here only ever reads the cache.
fn enclosing_node<'a>(
    specs: &'a HashMap<String, Option<Rc<Subcommand>>>,
    command: &Command,
    root: &'a Subcommand,
    node_index: usize,
) -> Option<&'a Subcommand> {
    if node_index == 0 {
        return None;
    }
    let prefix = Command {
        start: 0,
        end: 0,
        assignments: Vec::new(),
        words: command.words[..=node_index].to_vec(),
        terminator: None,
    };
    let walk = argwalk::walk(&prefix, root, |name| {
        specs.get(name).and_then(|found| found.as_deref())
    });
    Some(walk.node)
}

fn already_passed(passed: &[&Opt], candidate: &Rc<Opt>) -> bool {
    passed
        .iter()
        .any(|opt| std::ptr::eq(*opt, Rc::as_ptr(candidate)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Fixture;

    fn target(line: &str) -> Target {
        parse(line, "", &HashMap::new()).expect("a command this provider should answer for")
    }

    /// A reading of `$HISTFILE` that saw `first second` `n` times and
    /// nothing else at all.
    fn counts(first: &str, second: &str, n: u32) -> histfile::Counts {
        histfile::Counts(HashMap::from([(
            first.to_string(),
            HashMap::from([(second.to_string(), n)]),
        )]))
    }

    #[test]
    fn history_ranks_a_used_second_word_above_an_unused_one() {
        // `docker ps` is what this reading saw. `ps` sorts nowhere near
        // first among `docker`'s 58 subcommands and leads the menu anyway.
        let mut c = Completions {
            cmd_history: counts("docker", "ps", 100),
            ..Default::default()
        };
        let rows = complete(&mut c, &target("docker "));
        assert_eq!(names(&rows)[0], "ps", "{:?}", names(&rows));
    }

    #[test]
    fn history_leaves_a_word_deeper_than_it_counted_alone() {
        // `ls` here is `docker container`'s own third word and the reading
        // only ever counted a second. A count for `("docker", "ls")` is a
        // `docker ls` nobody typed, so this menu comes out in the order it
        // would have with no history at all.
        let mut counted = Completions {
            cmd_history: counts("docker", "ls", 100),
            ..Default::default()
        };
        let ranked = complete(&mut counted, &target("docker container "));
        let plain = complete(&mut Completions::default(), &target("docker container "));
        assert!(names(&plain).contains(&"ls"), "{:?}", names(&plain));
        assert_eq!(names(&ranked), names(&plain));
    }

    #[test]
    fn history_ranks_a_folder_the_command_was_given_before() {
        // `readme` leads this menu on its name alone. The folder row wears
        // a slash the person never typed, and the reading of `cat src`
        // still has to reach it.
        let f = Fixture::new(&["src", "readme*"]);
        let mut c = Completions {
            cmd_history: counts("cat", "src", 100),
            ..Default::default()
        };
        let rows = c.complete(
            &target("cat "),
            f.path(),
            &History::default(),
            &mut Scan::default(),
        );
        assert_eq!(names(&rows)[0], "src/", "{:?}", names(&rows));
    }

    #[test]
    fn a_typed_prefix_still_beats_a_used_row_that_does_not_match_it() {
        // `read` fuzzy-matches `ad` without leading with it. History never
        // moves it ahead of a name the typed prefix does lead with.
        let mut rows: Vec<Candidate> = ["add", "read"]
            .into_iter()
            .filter_map(|name| {
                row(
                    "ad",
                    name,
                    Cow::Borrowed(SUBCOMMAND_LABEL),
                    Vec::new(),
                    Kind::Command,
                    DEFAULT_PRIORITY,
                )
            })
            .collect();
        let cmd_history = counts("docker", "read", 100);
        rank(
            &mut rows,
            "ad",
            Some(UsedAfter {
                command: "docker",
                counts: &cmd_history,
            }),
        );
        assert_eq!(names(&rows), ["add", "read"]);
    }

    /// One ranking pass over rows a caller hands in with the option
    /// first. The order that comes back is therefore the sort's own rather
    /// than the one it was given.
    fn ranked(term: &str, rows: &[(&str, Kind, u8)]) -> Vec<String> {
        let mut rows: Vec<Candidate> = rows
            .iter()
            .filter_map(|(name, kind, priority)| {
                let label = match kind {
                    Kind::Option => OPTION_LABEL,
                    _ => SUBCOMMAND_LABEL,
                };
                row(
                    term,
                    name,
                    Cow::Borrowed(label),
                    Vec::new(),
                    *kind,
                    *priority,
                )
            })
            .collect();
        rank(&mut rows, term, None);
        rows.into_iter().map(|c| c.display).collect()
    }

    #[test]
    fn a_subcommand_leads_an_option_when_nothing_was_typed() {
        // An empty term scores the two the same and the name was then the
        // only key left. `-` sorts under every letter.
        let rows = [
            ("--color", Kind::Option, DEFAULT_PRIORITY),
            ("add", Kind::Command, DEFAULT_PRIORITY),
        ];
        assert_eq!(ranked("", &rows), ["add", "--color"]);
    }

    #[test]
    fn a_typed_dash_leaves_the_subcommands_out_altogether() {
        // A `-` is how a person asks for the options and the group order
        // is not what answers. A name with no `-` anywhere in it is no
        // subsequence match and never becomes a row at all.
        let rows = [
            ("--color", Kind::Option, DEFAULT_PRIORITY),
            ("add", Kind::Command, DEFAULT_PRIORITY),
        ];
        assert_eq!(ranked("-", &rows), ["--color"]);
    }

    #[test]
    fn a_closer_match_leads_whatever_group_it_came_from() {
        // `log` runs whole through `--log` from the character after its
        // dashes and reaches `dialog` three characters in. Neither name
        // leads with it. The score is what separates them and the group
        // breaks nothing here.
        let rows = [
            ("--log", Kind::Option, DEFAULT_PRIORITY),
            ("dialog", Kind::Command, DEFAULT_PRIORITY),
        ];
        assert_eq!(ranked("log", &rows), ["--log", "dialog"]);
    }

    #[test]
    fn a_priority_lifts_a_row_over_the_group_it_came_from() {
        // The group order is only what holds where nothing else speaks.
        // A number the specification wrote down is the specification
        // speaking.
        let rows = [
            ("--message", Kind::Option, 100),
            ("add", Kind::Command, DEFAULT_PRIORITY),
        ];
        assert_eq!(ranked("", &rows), ["--message", "add"]);
    }

    #[test]
    fn a_closer_match_still_leads_a_higher_priority() {
        // `dialog` is at the top of the range and `--log` is at the
        // middle of it. What was typed runs whole through `--log` from
        // the character after its dashes and reaches `dialog` three
        // characters in. The sort reads that first.
        let rows = [
            ("--log", Kind::Option, DEFAULT_PRIORITY),
            ("dialog", Kind::Command, 100),
        ];
        assert_eq!(ranked("log", &rows), ["--log", "dialog"]);
    }

    #[test]
    fn a_specifications_own_priority_orders_the_options() {
        // `svn commit ` is the case this reads for. The corpus puts `-m`
        // at 100, `--username` at 95 and `--password` at 94. It leaves
        // the eight that configure the connection at the default. The
        // alphabetical order buried the one flag the command cannot run
        // without under six of them.
        let rows = complete(&mut Completions::default(), &target("svn commit "));
        assert_eq!(
            names(&rows)[0..3],
            ["-m", "--username", "--password"],
            "{:?}",
            names(&rows)
        );
    }

    #[test]
    fn a_root_offers_every_subcommand_before_its_options() {
        // `cargo` carries 38 subcommands beside 12 options and the
        // options used to take the whole six-row window: `--color`
        // through `-V`.
        let rows = complete(&mut Completions::default(), &target("cargo "));
        let last_command = rows
            .iter()
            .rposition(|r| r.kind == Kind::Command)
            .expect("a subcommand row");
        let first_option = rows
            .iter()
            .position(|r| r.kind == Kind::Option)
            .expect("an option row");
        assert!(last_command < first_option, "{:?}", names(&rows));
        assert_eq!(names(&rows)[0], "add", "{:?}", names(&rows));
    }

    /// A `filepaths`/`folders` generator with no options of its own, the
    /// shape every real spec in the corpus uses today. `extra` adds the
    /// options a test wants on top of it.
    fn path_generator(extra: serde_json::Value) -> Generator {
        Generator {
            get_query_term: Some("/".to_string()),
            extra: extra.as_object().cloned().unwrap_or_default(),
            ..Generator::default()
        }
    }

    /// Every test below reads no real directory, so a path that cannot
    /// exist is cwd enough for the ones that never touch the filesystem.
    fn complete(c: &mut Completions, target: &Target) -> Vec<Candidate> {
        c.complete(
            target,
            Path::new("/no-such-directory-here"),
            &History::default(),
            &mut Scan::default(),
        )
    }

    #[test]
    fn a_command_name_is_loaded_once_and_reused() {
        let mut c = Completions::default();
        let first = c.spec_for("cargo").expect("cargo is a committed spec");
        let second = c.spec_for("cargo").expect("cargo is a committed spec");
        assert!(Rc::ptr_eq(&first, &second));
    }

    #[test]
    fn a_re_root_target_is_loaded_once_and_reused_across_menu_keys() {
        let mut c = Completions::default();
        complete(&mut c, &target("sudo git "));
        let first = c
            .specs
            .get("git")
            .expect("git was asked for")
            .clone()
            .expect("git is a committed spec");
        complete(&mut c, &target("sudo git switch "));
        let second = c
            .specs
            .get("git")
            .expect("git was asked for")
            .clone()
            .expect("git is a committed spec");
        // Only `sudo` and `git`: the second call re-asked for both and found
        // each one already cached, rather than loading either again.
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(c.specs.len(), 2);
    }

    #[test]
    fn cat_offers_files_and_folders_from_the_fixture_directory() {
        let f = Fixture::new(&["src", "readme*"]);
        std::fs::write(f.path().join("src").join("main.rs"), b"").unwrap();
        let mut c = Completions::default();
        let rows = c.complete(
            &target("cat "),
            f.path(),
            &History::default(),
            &mut Scan::default(),
        );
        let names: Vec<&str> = rows.iter().map(|r| r.display.as_str()).collect();
        assert!(names.contains(&"src/"), "{names:?}");
        assert!(names.contains(&"readme"), "{names:?}");
    }

    #[test]
    fn make_dash_c_offers_folders_and_no_files() {
        // `-C`'s own argument is a plain `template: "folders"`, with nothing
        // else on the arg or on `make`'s own `generators` list.
        let f = Fixture::new(&["src", "readme*"]);
        let mut c = Completions::default();
        let rows = c.complete(
            &target("make -C "),
            f.path(),
            &History::default(),
            &mut Scan::default(),
        );
        let names: Vec<&str> = rows.iter().map(|r| r.display.as_str()).collect();
        assert!(names.contains(&"src/"), "{names:?}");
        assert!(!names.contains(&"readme"), "{names:?}");
    }

    #[test]
    fn the_query_term_is_read_after_the_last_slash() {
        let f = Fixture::new(&["src"]);
        std::fs::write(f.path().join("src").join("main.rs"), b"").unwrap();
        let generator = path_generator(serde_json::json!({}));
        let rows = path_rows(
            &generator,
            "src/ma",
            f.path(),
            &History::default(),
            &mut Scan::default(),
            false,
        );
        let names: Vec<&str> = rows.iter().map(|r| r.display.as_str()).collect();
        assert_eq!(names, ["main.rs"]);
        assert_eq!(rows[0].insert, "src/main.rs");
    }

    #[test]
    fn a_folder_row_ends_in_a_slash() {
        let f = Fixture::new(&["assets"]);
        let generator = path_generator(serde_json::json!({}));
        let rows = path_rows(
            &generator,
            "",
            f.path(),
            &History::default(),
            &mut Scan::default(),
            false,
        );
        assert_eq!(rows[0].display, "assets/");
        assert_eq!(rows[0].insert, "assets/");
    }

    #[test]
    fn a_generator_without_a_slash_query_term_offers_nothing() {
        let generator = Generator::default();
        let rows = path_rows(
            &generator,
            "any",
            Path::new("/no-such-directory-here"),
            &History::default(),
            &mut Scan::default(),
            false,
        );
        assert!(rows.is_empty());
    }

    #[test]
    fn show_folders_never_drops_every_folder() {
        let f = Fixture::new(&["assets", "notes.txt*"]);
        let generator = path_generator(serde_json::json!({"showFolders": "never"}));
        let rows = path_rows(
            &generator,
            "",
            f.path(),
            &History::default(),
            &mut Scan::default(),
            false,
        );
        let names: Vec<&str> = rows.iter().map(|r| r.display.as_str()).collect();
        assert_eq!(names, ["notes.txt"]);
    }

    #[test]
    fn show_folders_only_drops_every_file() {
        let f = Fixture::new(&["assets", "notes.txt*"]);
        let generator = path_generator(serde_json::json!({"showFolders": "only"}));
        let rows = path_rows(
            &generator,
            "",
            f.path(),
            &History::default(),
            &mut Scan::default(),
            false,
        );
        let names: Vec<&str> = rows.iter().map(|r| r.display.as_str()).collect();
        assert_eq!(names, ["assets/"]);
    }

    #[test]
    fn a_folders_only_call_behaves_like_show_folders_only() {
        let f = Fixture::new(&["assets", "notes.txt*"]);
        let generator = path_generator(serde_json::json!({}));
        let rows = path_rows(
            &generator,
            "",
            f.path(),
            &History::default(),
            &mut Scan::default(),
            true,
        );
        let names: Vec<&str> = rows.iter().map(|r| r.display.as_str()).collect();
        assert_eq!(names, ["assets/"]);
    }

    #[test]
    fn the_scan_limit_holds_for_a_path_argument() {
        let mut entries: Vec<String> = (0..300).map(|i| format!("file-{i}*")).collect();
        entries.extend((0..300).map(|i| format!("dir-{i}")));
        let names: Vec<&str> = entries.iter().map(String::as_str).collect();
        let f = Fixture::new(&names);
        let generator = path_generator(serde_json::json!({}));
        let rows = path_rows(
            &generator,
            "",
            f.path(),
            &History::default(),
            &mut Scan::default(),
            false,
        );
        assert_eq!(rows.len(), crate::candidates::SCAN_LIMIT);
    }

    #[test]
    fn a_history_template_gives_no_rows_yet() {
        let generator = Generator {
            template: vec!["history".to_string()],
            ..Generator::default()
        };
        let rows = generator_rows(
            &generator,
            "",
            Path::new("/no-such-directory-here"),
            &History::default(),
            &mut Scan::default(),
            None,
        );
        assert!(rows.is_empty());
    }

    /// A fixture directory holding a makefile. Its targets are what the
    /// three tests below read; `make`'s own argument reaches the native
    /// reader through the same `cwd` a path argument already uses.
    fn make_fixture() -> Fixture {
        let f = Fixture::new(&[]);
        std::fs::write(
            f.path().join("Makefile"),
            ".PHONY: sample-check\nsample-build:\n\t:\nsample-check: sample-build\n\t:\n",
        )
        .unwrap();
        f
    }

    fn names(rows: &[Candidate]) -> Vec<&str> {
        rows.iter().map(|r| r.display.as_str()).collect()
    }

    #[test]
    fn make_offers_the_targets_of_the_makefile_beside_it() {
        let f = make_fixture();
        let mut c = Completions::default();
        let rows = c.complete(
            &target("make "),
            f.path(),
            &History::default(),
            &mut Scan::default(),
        );
        let names = names(&rows);
        assert!(names.contains(&"sample-build"), "{names:?}");
        assert!(names.contains(&"sample-check"), "{names:?}");
    }

    #[test]
    fn make_offers_the_targets_behind_an_option_of_its_own() {
        // `-j` takes a job count first and the same `target` argument
        // second, which is index 1 rather than index 0.
        let f = make_fixture();
        let mut c = Completions::default();
        let rows = c.complete(
            &target("make -j 4 "),
            f.path(),
            &History::default(),
            &mut Scan::default(),
        );
        let names = names(&rows);
        assert!(names.contains(&"sample-build"), "{names:?}");
        assert!(names.contains(&"sample-check"), "{names:?}");
    }

    #[test]
    fn make_offers_no_target_where_there_is_no_makefile() {
        let f = Fixture::new(&[]);
        let mut c = Completions::default();
        let rows = c.complete(
            &target("make "),
            f.path(),
            &History::default(),
            &mut Scan::default(),
        );
        assert!(
            rows.iter().all(|r| r.label != "target"),
            "{:?}",
            names(&rows)
        );
    }

    #[test]
    fn a_dynamic_argument_with_no_reader_still_offers_nothing() {
        // `chown`'s own first argument is `dyn` and no reader answers it.
        let mut c = Completions::default();
        let rows = complete(&mut c, &target("chown "));
        assert!(
            rows.iter().all(|r| r.kind == Kind::Option),
            "{:?}",
            names(&rows)
        );
    }

    #[test]
    fn help_offers_the_enclosing_nodes_siblings() {
        let mut c = Completions::default();
        let rows = complete(&mut c, &target("fnm help "));
        let names: Vec<&str> = rows
            .iter()
            .filter(|r| r.kind == Kind::Command)
            .map(|r| r.display.as_str())
            .collect();
        assert!(names.contains(&"install"), "{names:?}");
        assert!(names.contains(&"env"), "{names:?}");
    }

    #[test]
    fn help_reaches_a_node_nested_two_levels_deep() {
        let mut c = Completions::default();
        let rows = complete(&mut c, &target("shell-config external help "));
        let names: Vec<&str> = rows
            .iter()
            .filter(|r| r.kind == Kind::Command)
            .map(|r| r.display.as_str())
            .collect();
        assert!(names.contains(&"list"), "{names:?}");
        assert!(names.contains(&"install"), "{names:?}");
        assert!(names.contains(&"delete"), "{names:?}");
    }

    /// `specs/npm.json`'s `install` argument is `package`, `isOptional` and
    /// `isVariadic` together.
    #[test]
    fn a_subcommand_row_carries_its_argument_hint() {
        let mut c = Completions::default();
        let rows = complete(&mut c, &target("npm "));
        let install = rows
            .iter()
            .find(|r| r.insert == "install")
            .expect("npm has an install subcommand");
        assert_eq!(install.hint, vec!["[package...]"]);
    }
}
