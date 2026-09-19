//! Completion from a command's own specification.
//!
//! Where Git and `cd` each read their own syntax, this reads whatever
//! [`crate::spec`] has for the command name in front of the cursor. `App`
//! reaches for it only once neither of the other two has claimed the line.

use crate::argwalk::{self, Walk};
use crate::candidates::{Candidate, Kind, MAX_RESULTS, Query, match_rank};
use crate::fuzzy;
use crate::shellparse::{self, Command};
use crate::spec::{self, Opt, Subcommand};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
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
    let name = command.words[0].inner_text.as_str();
    // Only what Git's own menu and `cd`'s declined, never what they claim.
    if name == "git" || name == "cd" {
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
}

impl Completions {
    /// The rows `target` offers. `App::refresh` calls this only once `parse`
    /// has already found a command for the current line.
    pub(crate) fn complete(&mut self, target: &Target) -> Vec<Candidate> {
        let Some(root) = self.spec_for(&target.command.words[0].inner_text) else {
            return Vec::new();
        };
        self.walk_resolving(target, &root)
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
    fn walk_resolving(&mut self, target: &Target, root: &Rc<Subcommand>) -> Vec<Candidate> {
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
                return build_rows(&walk);
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

fn row(term: &str, name: &str, label: Cow<'static, str>, kind: Kind) -> Option<Candidate> {
    Some(Candidate {
        display: name.to_string(),
        insert: name.to_string(),
        label,
        kind,
        score: fuzzy::score(term, name)?,
    })
}

/// The rows one walk offers, ranked against [`Walk::search_term`].
fn build_rows(walk: &Walk) -> Vec<Candidate> {
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
                Kind::Command,
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
            rows.extend(row(
                term,
                name,
                label(&opt.description, OPTION_LABEL),
                Kind::Option,
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
                Kind::Path,
            ));
        }
    }

    rank(&mut rows, term);
    rows
}

fn already_passed(passed: &[&Opt], candidate: &Rc<Opt>) -> bool {
    passed
        .iter()
        .any(|opt| std::ptr::eq(*opt, Rc::as_ptr(candidate)))
}

/// The same ranking `git.rs` gives its own rows: a name that folds to
/// exactly what was typed leads, a name that merely starts with it follows,
/// ties keep the fuzzy score's own order and a name breaks any tie that is
/// still left.
fn rank(rows: &mut Vec<Candidate>, term: &str) {
    let tier = |name: &str| {
        if !term.is_empty()
            && fuzzy::starts_with_folded(name, term)
            && fuzzy::starts_with_folded(term, name)
        {
            2
        } else {
            match_rank(term, name)
        }
    };
    rows.sort_by(|a, b| {
        tier(&b.display)
            .cmp(&tier(&a.display))
            .then(b.score.cmp(&a.score))
            .then(a.display.cmp(&b.display))
    });
    rows.truncate(MAX_RESULTS);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(line: &str) -> Target {
        parse(line, "", &HashMap::new()).expect("a command this provider should answer for")
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
        c.complete(&target("sudo git "));
        let first = c
            .specs
            .get("git")
            .expect("git was asked for")
            .clone()
            .expect("git is a committed spec");
        c.complete(&target("sudo git switch "));
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
}
