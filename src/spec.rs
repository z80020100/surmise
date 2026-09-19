//! The normalized shape of a completion specification, and the loader that
//! turns `spec_store`'s bytes into it. `CLAUDE.md`'s "Completion spec data"
//! section says what `specs/` is; this module is what reads one file from it.
//!
//! **The committed data is the raw shape a spec author wrote.** `@fig/autocomplete-shared`
//! normalizes that shape at Q's own load time, and `plans/reference/q-inventory-npm.md`
//! §1 is that package read cover to cover. This module ports its rules:
//!
//! - `name` is always a list. A string becomes a one-element list. The same
//!   rule is applied here to every name-bearing field ([`Subcommand`], [`Opt`],
//!   [`Arg`] and [`Suggestion`] all declare `name` as `string | string[]`),
//!   not only to the subcommand the inventory's own text singles out.
//! - `subcommands` becomes a map from every name and alias to one shared
//!   node. The loop runs forward and assigns, so a later subcommand whose
//!   name collides with an earlier one wins. [`Stats::subcommand_collisions`]
//!   counts it instead of swallowing it.
//! - `options` splits on `isPersistent` into `options` and
//!   `persistent_options` on the same node. Nothing copies a persistent
//!   option into a child; the argument walk is a later unit's job.
//! - `args` is always a list, empty when absent.
//! - `parserDirectives` is inherited by a child only when the child declares
//!   none.
//! - `template` on an arg becomes one more entry in `generators`. Q's own
//!   `initializeDefault` *removes* the arg's own generators when it does
//!   this; surmise keeps both, because `gap-analysis.md` §6 item 14 found
//!   nothing in Q's own engine that depends on the discard, and the data
//!   disagrees for [8 arguments](Arg) out of 235 174.
//! - A generator whose template names `folders` or `filepaths` gets
//!   `trigger` and `get_query_term` of `/` unless the spec set them.
//! - `loadSpec` as a string becomes one [`LoadSpec`] of kind `"global"`. The
//!   committed data never uses the object or array forms the public type
//!   also allows, so this module does not model them.
//!
//! What surmise does not carry forward: `Callable(Handle)` is gone (no
//! runtime exists to call), a [`Generator`] holds data alone with no
//! `custom`, `postProcess` or `filterTemplateSuggestions`, and the
//! versioned-spec machinery is a single-hop pointer rather than a semver
//! walk. `plans/phase-1-parser.md`'s `spec` section is the fuller account.

use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;

use crate::spec_store;

/// A JSON object's fields with no place of their own on the struct that
/// parsed it. Every type in this module carries one, so a field a corpus
/// regeneration adds tomorrow still survives today's normalization instead
/// of being silently dropped.
pub type Extra = serde_json::Map<String, Value>;

/// Why [`load`] could not hand back a spec.
#[derive(Debug)]
pub enum SpecError {
    /// No committed spec answers this name. The caller's own answer to this
    /// is `PASS`, so the shell's own completion runs.
    Missing,
    /// The bytes `spec_store` handed back are not the shape a spec takes.
    /// The corpus is fixed and every committed spec parses, so this should
    /// only fire against a corrupted build.
    Malformed(serde_json::Error),
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SpecError::Missing => write!(f, "no committed spec answers this name"),
            SpecError::Malformed(err) => write!(f, "spec did not parse: {err}"),
        }
    }
}

impl std::error::Error for SpecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SpecError::Missing => None,
            SpecError::Malformed(err) => Some(err),
        }
    }
}

/// A completion spec's normalized root. A spec file's own top level is one
/// of these, and so is every entry under `subcommands`: the corpus and
/// `Fig.Subcommand` both give a spec the same shape as a subcommand of one.
#[derive(Debug, Clone)]
pub struct Subcommand {
    pub name: Vec<String>,
    pub subcommands: HashMap<String, Rc<Subcommand>>,
    pub options: HashMap<String, Rc<Opt>>,
    pub persistent_options: HashMap<String, Rc<Opt>>,
    pub args: Vec<Arg>,
    /// Resolved against every ancestor already, per the inheritance rule
    /// above. A later phase reads this field alone; it never has to walk
    /// back up the tree.
    pub parser_directives: Option<ParserDirectives>,
    pub load_spec: Vec<LoadSpec>,
    /// The bare boolean `Fig.Subcommand.cache` declares. Parsed and never
    /// read anywhere in Q; `Generator.cache` is the one anything acts on
    /// (`plans/reference/q-inventory-npm.md` §4).
    pub cache: Option<bool>,
    pub description: Option<String>,
    pub icon: Option<String>,
    /// The ranking chain's own input, 0..100 in every occurrence in the
    /// corpus. Clamping and defaulting to 50 is that chain's job, not this
    /// module's, so the raw value travels through unchanged.
    pub priority: Option<i64>,
    pub hidden: Option<bool>,
    pub display_name: Option<String>,
    pub insert_value: Option<String>,
    pub requires_subcommand: Option<bool>,
    /// `additionalSuggestions`, normalized the same way `Arg.suggestions`
    /// and `Generator.suggestions` are: a bare string becomes a one-name
    /// `Suggestion`.
    pub additional_suggestions: Vec<Suggestion>,
    pub is_dangerous: Option<bool>,
    pub filter_strategy: Option<String>,
    pub extra: Extra,
}

/// One of a subcommand's options. Named `Opt` because `Option` is
/// `std::option::Option` and shadowing it would make every signature in this
/// module read backwards.
#[derive(Debug, Clone)]
pub struct Opt {
    pub name: Vec<String>,
    pub args: Vec<Arg>,
    pub is_persistent: bool,
    /// `isRequired`. Parsed and never read anywhere in Q
    /// (`plans/reference/q-inventory-npm.md` §4).
    pub is_required: Option<bool>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub priority: Option<i64>,
    pub hidden: Option<bool>,
    pub display_name: Option<String>,
    pub insert_value: Option<String>,
    pub is_repeatable: Repeatable,
    pub requires_separator: Option<Separator>,
    pub requires_equals: Option<bool>,
    /// Names of options that remove this one from the suggestion list once
    /// any of them has been passed. Parsing is unaffected.
    pub exclusive_on: Vec<String>,
    /// Names of options this one depends on. An unmet dependency is not
    /// filtered out; it is boosted in ranking instead.
    pub depends_on: Vec<String>,
    pub is_dangerous: Option<bool>,
    pub extra: Extra,
}

/// A positional argument, to a subcommand or to an option.
#[derive(Debug, Clone)]
pub struct Arg {
    pub name: Vec<String>,
    pub suggestions: Vec<Suggestion>,
    pub generators: Vec<Generator>,
    /// `dyn` in the data, renamed because `dyn` is a Rust keyword. An
    /// argument that needed code to answer it carries this rather than a
    /// runtime handle; `specs/dynamic.txt` is the corpus-wide list.
    pub dynamic: bool,
    pub load_spec: Vec<LoadSpec>,
    /// `Fig.Arg.default`. Parsed and never read anywhere in Q; its shape
    /// varies with the argument, so it stays a raw JSON value rather than a
    /// guessed Rust type.
    pub default: Option<Value>,
    pub description: Option<String>,
    pub is_optional: Option<bool>,
    pub is_variadic: Option<bool>,
    pub options_can_break_variadic_arg: Option<bool>,
    pub is_command: Option<bool>,
    pub is_script: Option<bool>,
    /// The module prefix `isModule` carries, e.g. `python`'s `python/` on
    /// `python -m`. Unlike `isCommand` and `isScript` this is a string, not
    /// a flag.
    pub is_module: Option<String>,
    pub filter_strategy: Option<String>,
    pub suggest_current_token: Option<bool>,
    pub is_dangerous: Option<bool>,
    /// `Fig.Arg.debounce`, read by the argument walk's trigger logic
    /// (`plans/reference/q-inventory-parser.md` §1.3 and §4.4). This module
    /// only carries it through; it does not act on it.
    pub debounce: Option<bool>,
    pub extra: Extra,
}

/// `Fig.Option.isRepeatable`: `false` or absent means once, `true` means
/// unlimited, and a number is the cap. Three shapes in one field, so this
/// gets a type that says so rather than a raw JSON value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Repeatable {
    #[default]
    Once,
    Unlimited,
    Times(u32),
}

impl<'de> Deserialize<'de> for Repeatable {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Flag(bool),
            Count(u32),
        }
        Ok(match Raw::deserialize(deserializer)? {
            Raw::Flag(true) => Repeatable::Unlimited,
            Raw::Flag(false) => Repeatable::Once,
            Raw::Count(count) => Repeatable::Times(count),
        })
    }
}

/// `Fig.Option.requiresSeparator`: `true` asks for the option's own
/// separator, the first of `parserDirectives.optionArgSeparators`, or `=`,
/// in that order; an explicit string overrides the whole resolution.
/// `false` never appears in the corpus, so `Option<Separator>` alone (with
/// `None` for both "absent" and, were it ever written, "false") covers
/// every case the data has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Separator {
    Default,
    Explicit(String),
}

/// Where an argument's suggestions come from, holding data alone. Q's own
/// `Fig.Generator` also carries a `custom` callback, a `postProcess`
/// callback and a `filterTemplateSuggestions` callback; none of the three
/// survives conversion into the corpus, so none has a place here.
#[derive(Debug, Clone, Default)]
pub struct Generator {
    pub template: Vec<String>,
    /// A command line: a string, a list of arguments, or an object carrying
    /// `command`/`args`/`cwd`/`env`. Left as JSON because interpreting it is
    /// `plans/phase-2-spec-runtime.md` §4's job, not this module's.
    pub script: Option<Value>,
    pub split_on: Option<String>,
    /// Every `trigger` in the committed data is the string form; the
    /// function form cannot survive conversion and the object form never
    /// appears.
    pub trigger: Option<String>,
    pub get_query_term: Option<String>,
    pub filter_strategy: Option<String>,
    pub suggestions: Vec<Suggestion>,
    pub cache: Option<GeneratorCache>,
    /// `Fig.Generator.debounce`. Parsed and never read: only `Fig.Arg.debounce`
    /// is (`plans/reference/q-inventory-parser.md` §1.4).
    pub debounce: Option<Value>,
    pub extra: Extra,
}

/// `Generator.cache`. `strategy`, `ttl` and `cacheByDirectory` are read by a
/// later phase; `cacheKey` only ever appears as a plain string in the
/// corpus, never as the function form the public type also allows.
#[derive(Debug, Clone)]
pub struct GeneratorCache {
    pub strategy: Option<String>,
    pub ttl: Option<u64>,
    pub cache_by_directory: Option<bool>,
    pub cache_key: Option<String>,
    pub extra: Extra,
}

/// A candidate a menu can show: a static entry in `Arg.suggestions` or
/// `Generator.suggestions`, or one of a subcommand's `additionalSuggestions`.
/// The corpus writes many of these as a bare string; [`Arg`] and
/// [`Generator`] normalize that into a one-name `Suggestion` at the point
/// they read their own suggestion lists.
#[derive(Debug, Clone, Default)]
pub struct Suggestion {
    pub name: Vec<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub priority: Option<i64>,
    pub hidden: Option<bool>,
    pub display_name: Option<String>,
    pub insert_value: Option<String>,
    /// `type` in the data, renamed because `type` is a Rust keyword.
    pub kind: Option<String>,
    pub is_dangerous: Option<bool>,
    /// Parsed and never read anywhere in Q
    /// (`plans/reference/q-inventory-npm.md` §4).
    pub deprecated: Option<bool>,
    /// Parsed and never read: insertion reads `insertValue` alone
    /// (`plans/reference/q-inventory-npm.md` §4).
    pub replace_value: Option<String>,
    /// `_internal`. Parsed and never read
    /// (`plans/reference/q-inventory-npm.md` §4).
    pub internal: Option<Value>,
    pub extra: Extra,
}

/// `parserDirectives`, already resolved against every ancestor by the time
/// it reaches a [`Subcommand`]. `alias` is not modelled: it is a function in
/// every spec that declares one, so it never survives conversion into data
/// (`plans/phase-1-parser.md`'s `argwalk` section).
#[derive(Debug, Clone, Default)]
pub struct ParserDirectives {
    pub flags_are_posix_noncompliant: bool,
    pub options_must_precede_arguments: bool,
    pub option_arg_separators: Vec<String>,
    pub extra: Extra,
}

/// One location `loadSpec` names. `kind` is always `"global"`: that is what
/// `initializeDefault` gives a string-form `loadSpec`, and the corpus never
/// uses the object or array forms that could carry a different one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadSpec {
    pub name: String,
    pub kind: String,
}

/// Counts a normalization pass measured, so a test can assert them against
/// the converter's own report rather than trust that the walk found
/// everything. `plans/phase-1-parser.md`'s exit criteria name the corpus
/// totals this is checked against.
#[derive(Debug, Default, Clone, Copy)]
struct Stats {
    subcommands: usize,
    options: usize,
    args: usize,
    /// How many times building a `subcommands` map overwrote an entry a
    /// name or alias earlier in the same list had already claimed.
    subcommand_collisions: usize,
}

/// Loads and normalizes the committed spec named `name`. A file whose only
/// fields are `name` and `versionedSpecPath` is a pointer; it is followed
/// once; `plans/phase-2-spec-runtime.md` §1 confirms a pointer never points
/// at another.
///
/// This performs no session-level caching. `plans/phase-2-spec-runtime.md`
/// §1 owns that, on top of the walk this function exists to provide.
///
/// `spec_dirs` is taken as a parameter for the reason `spec_store` gives for
/// the same choice: a `static` holding the config can be set once, and a
/// test binary runs every test in one process, so the first test to seed it
/// would pin the directories for all the rest. [`load_configured`] is the
/// process-wide entry point a picker uses.
pub fn load(name: &str, spec_dirs: &[PathBuf]) -> Result<Subcommand, SpecError> {
    load_with(name, |n| read_raw(n, spec_dirs))
}

/// [`load`] over the config this process was started with, so a name in
/// `disabled_commands` is refused and `spec_dirs` is searched first. This is
/// the entry point a picker uses; nothing calls it yet, because nothing
/// completes from a spec until `plans/phase-2-spec-runtime.md` lands.
pub fn load_configured(name: &str) -> Result<Subcommand, SpecError> {
    load_with(name, read_raw_configured)
}

/// The two entry points differ only in where the bytes come from. A pointer
/// is followed with the same reader that found it, once and no further.
fn load_with(
    name: &str,
    read: impl Fn(&str) -> Result<RawSubcommand, SpecError>,
) -> Result<Subcommand, SpecError> {
    let raw = read(name)?;
    let raw = match &raw.versioned_spec_path {
        Some(target) => read(target)?,
        None => raw,
    };
    let mut stats = Stats::default();
    Ok(normalize_subcommand(raw, None, &mut stats))
}

fn read_raw(name: &str, spec_dirs: &[PathBuf]) -> Result<RawSubcommand, SpecError> {
    parse_raw(spec_store::get(name, spec_dirs))
}

fn read_raw_configured(name: &str) -> Result<RawSubcommand, SpecError> {
    parse_raw(spec_store::get_configured(name))
}

fn parse_raw(bytes: Option<Vec<u8>>) -> Result<RawSubcommand, SpecError> {
    let bytes = bytes.ok_or(SpecError::Missing)?;
    serde_json::from_slice(&bytes).map_err(SpecError::Malformed)
}

/// Builds the `name → shared node` map `convertSubcommand` builds for
/// `subcommands`, `options` and `persistentOptions` alike: every alias of
/// `entries[i]` becomes a key over the same `Rc`, and inserting a name a
/// prior entry already claimed overwrites it. The loop runs in the data's
/// own order, so a later entry wins exactly as it does in Q.
fn build_map<T>(entries: Vec<(Vec<String>, T)>) -> (HashMap<String, Rc<T>>, usize) {
    let mut map = HashMap::new();
    let mut collisions = 0;
    for (names, value) in entries {
        let shared = Rc::new(value);
        for name in names {
            if map.insert(name, Rc::clone(&shared)).is_some() {
                collisions += 1;
            }
        }
    }
    (map, collisions)
}

fn normalize_subcommand(
    raw: RawSubcommand,
    inherited: Option<&ParserDirectives>,
    stats: &mut Stats,
) -> Subcommand {
    let parser_directives = match raw.parser_directives {
        Some(directives) => Some(normalize_parser_directives(directives)),
        None => inherited.cloned(),
    };

    let mut named_subcommands = Vec::with_capacity(raw.subcommands.len());
    for child in raw.subcommands {
        stats.subcommands += 1;
        let names = child.name.clone();
        named_subcommands.push((
            names,
            normalize_subcommand(child, parser_directives.as_ref(), stats),
        ));
    }
    let (subcommands, collisions) = build_map(named_subcommands);
    stats.subcommand_collisions += collisions;

    let mut named_options = Vec::new();
    let mut named_persistent_options = Vec::new();
    for opt in raw.options {
        stats.options += 1;
        let names = opt.name.clone();
        let persistent = opt.is_persistent;
        let normalized = normalize_option(opt, stats);
        if persistent {
            named_persistent_options.push((names, normalized));
        } else {
            named_options.push((names, normalized));
        }
    }
    let (options, _) = build_map(named_options);
    let (persistent_options, _) = build_map(named_persistent_options);

    Subcommand {
        name: raw.name,
        subcommands,
        options,
        persistent_options,
        args: raw
            .args
            .into_iter()
            .map(|arg| normalize_arg(arg, stats))
            .collect(),
        parser_directives,
        load_spec: normalize_load_spec(raw.load_spec),
        cache: raw.cache,
        description: raw.description,
        icon: raw.icon,
        priority: raw.priority,
        hidden: raw.hidden,
        display_name: raw.display_name,
        insert_value: raw.insert_value,
        requires_subcommand: raw.requires_subcommand,
        additional_suggestions: raw
            .additional_suggestions
            .into_iter()
            .map(normalize_suggestion)
            .collect(),
        is_dangerous: raw.is_dangerous,
        filter_strategy: raw.filter_strategy,
        extra: raw.extra,
    }
}

fn normalize_option(raw: RawOpt, stats: &mut Stats) -> Opt {
    Opt {
        name: raw.name,
        args: raw
            .args
            .into_iter()
            .map(|arg| normalize_arg(arg, stats))
            .collect(),
        is_persistent: raw.is_persistent,
        is_required: raw.is_required,
        description: raw.description,
        icon: raw.icon,
        priority: raw.priority,
        hidden: raw.hidden,
        display_name: raw.display_name,
        insert_value: raw.insert_value,
        is_repeatable: raw.is_repeatable,
        requires_separator: raw.requires_separator,
        requires_equals: raw.requires_equals,
        exclusive_on: raw.exclusive_on,
        depends_on: raw.depends_on,
        is_dangerous: raw.is_dangerous,
        extra: raw.extra,
    }
}

fn normalize_arg(raw: RawArg, stats: &mut Stats) -> Arg {
    stats.args += 1;

    let mut generators: Vec<Generator> = raw
        .generators
        .into_iter()
        .map(normalize_generator)
        .collect();
    // Q's own `initializeDefault` removes `arg.generators` when it does
    // this; surmise keeps both. The module doc comment says why.
    if !raw.template.is_empty() {
        generators.push(Generator {
            template: raw.template,
            ..Generator::default()
        });
    }
    for generator in &mut generators {
        apply_path_defaults(generator);
    }

    Arg {
        name: raw.name,
        suggestions: raw
            .suggestions
            .into_iter()
            .map(normalize_suggestion)
            .collect(),
        generators,
        dynamic: raw.dynamic,
        load_spec: normalize_load_spec(raw.load_spec),
        default: raw.default,
        description: raw.description,
        is_optional: raw.is_optional,
        is_variadic: raw.is_variadic,
        options_can_break_variadic_arg: raw.options_can_break_variadic_arg,
        is_command: raw.is_command,
        is_script: raw.is_script,
        is_module: raw.is_module,
        filter_strategy: raw.filter_strategy,
        suggest_current_token: raw.suggest_current_token,
        is_dangerous: raw.is_dangerous,
        debounce: raw.debounce,
        extra: raw.extra,
    }
}

/// A generator whose own template names `folders` or `filepaths` re-queries
/// per path segment, unless the spec already set the two fields that make it
/// do that.
fn apply_path_defaults(generator: &mut Generator) {
    let is_path_template = generator
        .template
        .iter()
        .any(|name| name == "folders" || name == "filepaths");
    if !is_path_template {
        return;
    }
    if generator.trigger.is_none() {
        generator.trigger = Some("/".to_string());
    }
    if generator.get_query_term.is_none() {
        generator.get_query_term = Some("/".to_string());
    }
}

fn normalize_generator(raw: RawGenerator) -> Generator {
    Generator {
        template: raw.template,
        script: raw.script,
        split_on: raw.split_on,
        trigger: raw.trigger,
        get_query_term: raw.get_query_term,
        filter_strategy: raw.filter_strategy,
        suggestions: raw
            .suggestions
            .into_iter()
            .map(normalize_suggestion)
            .collect(),
        cache: raw.cache.map(normalize_cache),
        debounce: raw.debounce,
        extra: raw.extra,
    }
}

fn normalize_cache(raw: RawGeneratorCache) -> GeneratorCache {
    GeneratorCache {
        strategy: raw.strategy,
        ttl: raw.ttl,
        cache_by_directory: raw.cache_by_directory,
        cache_key: raw.cache_key,
        extra: raw.extra,
    }
}

fn normalize_suggestion(raw: RawSuggestion) -> Suggestion {
    Suggestion {
        name: raw.name,
        description: raw.description,
        icon: raw.icon,
        priority: raw.priority,
        hidden: raw.hidden,
        display_name: raw.display_name,
        insert_value: raw.insert_value,
        kind: raw.kind,
        is_dangerous: raw.is_dangerous,
        deprecated: raw.deprecated,
        replace_value: raw.replace_value,
        internal: raw.internal,
        extra: raw.extra,
    }
}

fn normalize_parser_directives(raw: RawParserDirectives) -> ParserDirectives {
    ParserDirectives {
        flags_are_posix_noncompliant: raw.flags_are_posix_noncompliant,
        options_must_precede_arguments: raw.options_must_precede_arguments,
        option_arg_separators: raw.option_arg_separators,
        extra: raw.extra,
    }
}

fn normalize_load_spec(raw: Option<String>) -> Vec<LoadSpec> {
    raw.into_iter()
        .map(|name| LoadSpec {
            name,
            kind: "global".to_string(),
        })
        .collect()
}

// --- The raw shape, exactly as a spec author wrote it. ---
//
// `serde(flatten)` on `extra` is what makes every field spread through
// untouched: whatever a named field on the struct does not claim lands in
// the map instead of being dropped.

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawSubcommand {
    #[serde(default, deserialize_with = "one_or_many")]
    name: Vec<String>,
    #[serde(default)]
    subcommands: Vec<RawSubcommand>,
    #[serde(default)]
    options: Vec<RawOpt>,
    #[serde(default, deserialize_with = "one_or_many")]
    args: Vec<RawArg>,
    #[serde(default)]
    parser_directives: Option<RawParserDirectives>,
    #[serde(default)]
    load_spec: Option<String>,
    #[serde(default)]
    versioned_spec_path: Option<String>,
    #[serde(default)]
    cache: Option<bool>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    insert_value: Option<String>,
    #[serde(default)]
    requires_subcommand: Option<bool>,
    #[serde(default, deserialize_with = "suggestion_list")]
    additional_suggestions: Vec<RawSuggestion>,
    #[serde(default)]
    is_dangerous: Option<bool>,
    #[serde(default)]
    filter_strategy: Option<String>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawOpt {
    #[serde(default, deserialize_with = "one_or_many")]
    name: Vec<String>,
    #[serde(default, deserialize_with = "one_or_many")]
    args: Vec<RawArg>,
    #[serde(default)]
    is_persistent: bool,
    #[serde(default)]
    is_required: Option<bool>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    insert_value: Option<String>,
    #[serde(default)]
    is_repeatable: Repeatable,
    #[serde(default, deserialize_with = "requires_separator")]
    requires_separator: Option<Separator>,
    #[serde(default)]
    requires_equals: Option<bool>,
    #[serde(default)]
    exclusive_on: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    is_dangerous: Option<bool>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawArg {
    #[serde(default, deserialize_with = "one_or_many")]
    name: Vec<String>,
    #[serde(default, deserialize_with = "suggestion_list")]
    suggestions: Vec<RawSuggestion>,
    #[serde(default, deserialize_with = "one_or_many")]
    generators: Vec<RawGenerator>,
    #[serde(default, deserialize_with = "one_or_many")]
    template: Vec<String>,
    #[serde(default, rename = "dyn")]
    dynamic: bool,
    #[serde(default)]
    load_spec: Option<String>,
    #[serde(default)]
    default: Option<Value>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    is_optional: Option<bool>,
    #[serde(default)]
    is_variadic: Option<bool>,
    #[serde(default)]
    options_can_break_variadic_arg: Option<bool>,
    #[serde(default)]
    is_command: Option<bool>,
    #[serde(default)]
    is_script: Option<bool>,
    #[serde(default)]
    is_module: Option<String>,
    #[serde(default)]
    filter_strategy: Option<String>,
    #[serde(default)]
    suggest_current_token: Option<bool>,
    #[serde(default)]
    is_dangerous: Option<bool>,
    #[serde(default)]
    debounce: Option<bool>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawGenerator {
    #[serde(default, deserialize_with = "one_or_many")]
    template: Vec<String>,
    #[serde(default)]
    script: Option<Value>,
    #[serde(default)]
    split_on: Option<String>,
    #[serde(default)]
    trigger: Option<String>,
    #[serde(default)]
    get_query_term: Option<String>,
    #[serde(default)]
    filter_strategy: Option<String>,
    #[serde(default, deserialize_with = "suggestion_list")]
    suggestions: Vec<RawSuggestion>,
    #[serde(default)]
    cache: Option<RawGeneratorCache>,
    #[serde(default)]
    debounce: Option<Value>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawGeneratorCache {
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default)]
    ttl: Option<u64>,
    #[serde(default)]
    cache_by_directory: Option<bool>,
    #[serde(default)]
    cache_key: Option<String>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawSuggestion {
    #[serde(default, deserialize_with = "one_or_many")]
    name: Vec<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
    #[serde(default)]
    hidden: Option<bool>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    insert_value: Option<String>,
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    is_dangerous: Option<bool>,
    #[serde(default)]
    deprecated: Option<bool>,
    #[serde(default)]
    replace_value: Option<String>,
    #[serde(default, rename = "_internal")]
    internal: Option<Value>,
    #[serde(flatten)]
    extra: Extra,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RawParserDirectives {
    #[serde(default)]
    flags_are_posix_noncompliant: bool,
    #[serde(default)]
    options_must_precede_arguments: bool,
    #[serde(default, deserialize_with = "one_or_many")]
    option_arg_separators: Vec<String>,
    #[serde(flatten)]
    extra: Extra,
}

/// Every name-bearing and template-bearing field in the corpus is declared
/// `T | T[]`. This reads either shape into the list form the rest of the
/// module works with, with the empty list left to the field's own
/// `#[serde(default)]` for when the key is absent entirely.
fn one_or_many<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany<T> {
        One(T),
        Many(Vec<T>),
    }
    Ok(match OneOrMany::<T>::deserialize(deserializer)? {
        OneOrMany::One(value) => vec![value],
        OneOrMany::Many(values) => values,
    })
}

/// A suggestion list holds a mix of bare strings and full objects. A bare
/// string is the name of a one-field `Fig.Suggestion`, so it collapses into
/// the same `RawSuggestion` shape the object form already has.
fn suggestion_list<'de, D>(deserializer: D) -> Result<Vec<RawSuggestion>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Item {
        Name(String),
        // Boxed so a bare-string suggestion, the far more common case,
        // does not pay for the full struct's size in every enum value.
        Full(Box<RawSuggestion>),
    }
    let items: Vec<Item> = one_or_many(deserializer)?;
    Ok(items
        .into_iter()
        .map(|item| match item {
            Item::Name(name) => RawSuggestion {
                name: vec![name],
                ..RawSuggestion::default()
            },
            Item::Full(suggestion) => *suggestion,
        })
        .collect())
}

/// `requiresSeparator`'s two live shapes. `false` never appears in the
/// corpus; this still folds it into `None` rather than a distinct
/// `Some(Separator::Default(false))` state, because "no separator required"
/// is exactly what an absent key already means.
fn requires_separator<'de, D>(deserializer: D) -> Result<Option<Separator>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Flag(bool),
        Value(String),
    }
    Ok(match Option::<Raw>::deserialize(deserializer)? {
        None | Some(Raw::Flag(false)) => None,
        Some(Raw::Flag(true)) => Some(Separator::Default),
        Some(Raw::Value(separator)) => Some(Separator::Explicit(separator)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::time::Instant;

    /// Every key `build.rs` embeds, computed the same way it computes them:
    /// a spec file's path under `specs/` relative to `specs/` itself, with
    /// the extension dropped. `specs/index.json` is the command list rather
    /// than a spec, so it is left out, exactly as `build.rs` leaves it out.
    fn spec_keys() -> Vec<String> {
        let mut keys = Vec::new();
        collect(Path::new("specs"), Path::new("specs"), &mut keys);
        keys
    }

    fn collect(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(dir).expect("read a specs/ directory") {
            let path = entry.expect("read a directory entry").path();
            if path.is_dir() {
                collect(&path, base, out);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "json") || path == base.join("index.json") {
                continue;
            }
            let key = path
                .strip_prefix(base)
                .expect("spec path is under specs/")
                .with_extension("")
                .to_str()
                .expect("spec path is UTF-8")
                .to_owned();
            out.push(key);
        }
    }

    #[test]
    fn every_committed_spec_loads() {
        for key in spec_keys() {
            load(&key, &[]).unwrap_or_else(|err| panic!("{key} did not load: {err}"));
        }
    }

    /// The corpus totals `tools/spec-convert` reported, and the phase-1 exit
    /// criterion at the struct level: reading raw nodes with `read_raw`
    /// rather than through `load` is what keeps a pointer's own stub (empty
    /// besides `name` and `versionedSpecPath`) from being counted twice,
    /// once under its own key and once under the key that points at it.
    #[test]
    fn corpus_totals_match_the_converters_report() {
        let mut stats = Stats::default();
        for key in spec_keys() {
            let raw =
                read_raw(&key, &[]).unwrap_or_else(|err| panic!("{key} did not parse: {err}"));
            normalize_subcommand(raw, None, &mut stats);
        }
        assert_eq!(stats.subcommands, 50_784, "subcommands");
        assert_eq!(stats.options, 282_837, "options");
        assert_eq!(stats.args, 235_174, "args");
        assert_eq!(stats.subcommand_collisions, 6, "subcommand name collisions");
    }

    /// `specs/trivy.json`'s `client` subcommand lists four options twice
    /// over, verbatim, under `subcommands` rather than `options`. That is
    /// real corpus data and not a fixture: it is the entire source of the
    /// six collisions the count above asserts.
    #[test]
    fn a_subcommand_name_collision_keeps_the_later_node() {
        let spec = load("trivy", &[]).expect("trivy is a committed spec");
        let client = spec
            .subcommands
            .get("client")
            .expect("trivy has a client subcommand");
        assert!(client.subcommands.contains_key("--severity"));
    }

    #[test]
    fn a_bare_string_name_becomes_a_one_element_list() {
        let spec = load("adb", &[]).expect("adb is a committed spec");
        assert_eq!(spec.name, vec!["adb".to_string()]);
    }

    /// `specs/az/2.53.0.json`'s top-level `--output` option carries the
    /// alias `-o`, and both are persistent.
    #[test]
    fn an_array_name_keeps_every_alias() {
        let spec = load("az", &[]).expect("az resolves through its pointer");
        let by_long = spec
            .persistent_options
            .get("--output")
            .expect("az --output is persistent");
        let by_short = spec
            .persistent_options
            .get("-o")
            .expect("az -o is the same option");
        assert!(
            Rc::ptr_eq(by_long, by_short),
            "both names must reach the same shared node"
        );
        assert_eq!(by_long.name, vec!["--output".to_string(), "-o".to_string()]);
    }

    #[test]
    fn persistent_options_split_out_and_do_not_reach_a_child() {
        let spec = load("az", &[]).expect("az resolves through its pointer");
        assert!(spec.persistent_options.contains_key("--debug"));
        assert!(!spec.options.contains_key("--debug"));
        for child in spec.subcommands.values() {
            assert!(
                !child.options.contains_key("--debug")
                    && !child.persistent_options.contains_key("--debug"),
                "a persistent option must not be copied into a child"
            );
        }
    }

    #[test]
    fn args_are_a_list_even_when_the_spec_has_none() {
        let spec = load("adb", &[]).expect("adb is a committed spec");
        let devices = spec
            .subcommands
            .get("devices")
            .expect("adb has a devices subcommand");
        assert!(devices.args.is_empty());
    }

    /// `specs/npm.json` sets `flagsArePosixNoncompliant` at the root and its
    /// `audit` subcommand declares no `parserDirectives` of its own.
    #[test]
    fn parser_directives_are_inherited_until_a_child_declares_its_own() {
        let spec = load("npm", &[]).expect("npm is a committed spec");
        assert!(
            spec.parser_directives
                .as_ref()
                .unwrap()
                .flags_are_posix_noncompliant
        );
        let audit = spec
            .subcommands
            .get("audit")
            .expect("npm has an audit subcommand");
        assert!(
            audit
                .parser_directives
                .as_ref()
                .unwrap()
                .flags_are_posix_noncompliant,
            "a child with no parserDirectives of its own inherits the parent's"
        );
    }

    /// `specs/mount.json`'s `Disk/loopfile` argument is one of the 8 where
    /// the corpus disagrees with Q's own discard: it carries a `template`
    /// alongside two script generators of its own, and surmise keeps all
    /// three.
    #[test]
    fn a_template_adds_a_generator_without_discarding_the_arg_s_own() {
        let spec = load("mount", &[]).expect("mount is a committed spec");
        let arg = spec
            .args
            .iter()
            .find(|arg| arg.name == vec!["Disk/loopfile".to_string()])
            .expect("mount has a Disk/loopfile argument");
        assert_eq!(
            arg.generators.len(),
            3,
            "two script generators plus the template"
        );
        assert!(
            arg.generators
                .iter()
                .any(|g| g.template == vec!["filepaths".to_string()])
        );
    }

    /// The one field in `q-inventory-npm.md` §1's table with a real example
    /// in the corpus: `specs/adb.json`'s `-s` option takes a `SERIAL` arg
    /// with a plain name and no template, so no generator is synthesized.
    #[test]
    fn an_arg_without_a_template_gets_no_synthesized_generator() {
        let spec = load("adb", &[]).expect("adb is a committed spec");
        let serial = spec.options.get("-s").expect("adb has a -s option");
        assert_eq!(serial.args.len(), 1);
        assert!(serial.args[0].generators.is_empty());
    }

    /// `specs/trivy.json`'s `client` subcommand carries `--skip-dirs` twice
    /// over, misfiled under `subcommands` rather than `options` (the same
    /// data behind [`a_subcommand_name_collision_keeps_the_later_node`]).
    /// Its `skipDirs` argument declares `template: "folders"` with neither
    /// `trigger` nor `getQueryTerm` set.
    #[test]
    fn a_folders_template_defaults_its_trigger_and_query_term() {
        let spec = load("trivy", &[]).expect("trivy is a committed spec");
        let client = spec
            .subcommands
            .get("client")
            .expect("trivy has a client subcommand");
        let skip_dirs = client
            .subcommands
            .get("--skip-dirs")
            .expect("trivy client has --skip-dirs");
        let generator = skip_dirs.args[0]
            .generators
            .iter()
            .find(|g| g.template == vec!["folders".to_string()])
            .expect("the folders template synthesized a generator");
        assert_eq!(generator.trigger.as_deref(), Some("/"));
        assert_eq!(generator.get_query_term.as_deref(), Some("/"));
    }

    /// `specs/pnpx.json`'s `create-react-native-app` subcommand declares
    /// `loadSpec` as a plain string.
    #[test]
    fn a_string_loadspec_on_a_node_becomes_one_global_entry() {
        let spec = load("pnpx", &[]).expect("pnpx is a committed spec");
        let child = spec
            .subcommands
            .get("create-react-native-app")
            .expect("pnpx has a create-react-native-app subcommand");
        assert_eq!(
            child.load_spec,
            vec![LoadSpec {
                name: "create-react-native-app".to_string(),
                kind: "global".to_string(),
            }]
        );
    }

    /// `specs/herd.json`'s `php` subcommand has a `command` argument that
    /// declares `loadSpec` as a plain string; this is the corpus's one
    /// argument-level occurrence.
    #[test]
    fn a_string_loadspec_on_an_arg_becomes_one_global_entry() {
        let spec = load("herd", &[]).expect("herd is a committed spec");
        let php = spec
            .subcommands
            .get("php")
            .expect("herd has a php subcommand");
        let command = php
            .args
            .iter()
            .find(|arg| arg.name == vec!["command".to_string()])
            .expect("php has a command argument");
        assert_eq!(
            command.load_spec,
            vec![LoadSpec {
                name: "php".to_string(),
                kind: "global".to_string(),
            }]
        );
    }

    /// The pointer case. `specs/az/index.json` is a two-field stub; the spec
    /// behind it, `specs/az/2.53.0.json`, is 34 475 bytes with real content:
    /// 228 subcommands and 7 options, every one of them persistent.
    #[test]
    fn a_pointer_resolves_to_the_whole_spec_behind_it() {
        let spec = load("az", &[]).expect("az's pointer resolves");
        assert_eq!(spec.subcommands.len(), 228);
        assert!(
            spec.options.is_empty(),
            "every top-level az option is persistent"
        );
        assert_eq!(
            spec.persistent_options.len(),
            9,
            "7 options, two of them with one alias each"
        );
    }

    #[test]
    fn a_missing_spec_is_the_not_ours_error() {
        match load("not-a-real-command-surmise-made-up", &[]) {
            Err(SpecError::Missing) => {}
            other => panic!("expected SpecError::Missing, got {other:?}"),
        }
    }

    /// `specs/onboardbase.json`'s `config:set-token` subcommand has a
    /// `--scope`/`-S` option marked `isRequired`, one of 17 219 real
    /// occurrences in the corpus.
    #[test]
    fn option_is_required_is_kept_and_never_read() {
        let spec = load("onboardbase", &[]).expect("onboardbase is a committed spec");
        let set_token = spec
            .subcommands
            .get("config:set-token")
            .expect("onboardbase has a config:set-token subcommand");
        let scope = set_token
            .options
            .get("--scope")
            .expect("config:set-token has --scope");
        assert_eq!(scope.is_required, Some(true));
    }

    #[test]
    fn generator_debounce_is_kept_and_never_read() {
        let raw: RawGenerator = serde_json::from_str(r#"{"script": "ls", "debounce": true}"#)
            .expect("valid generator JSON");
        let generator = normalize_generator(raw);
        assert_eq!(generator.debounce, Some(Value::Bool(true)));
    }

    #[test]
    fn subcommand_cache_boolean_is_kept_and_never_read() {
        let raw: RawSubcommand =
            serde_json::from_str(r#"{"name": "x", "cache": true}"#).expect("valid subcommand JSON");
        let mut stats = Stats::default();
        let spec = normalize_subcommand(raw, None, &mut stats);
        assert_eq!(spec.cache, Some(true));
    }

    #[test]
    fn suggestion_internal_and_replace_value_are_kept_and_never_read() {
        let raw: RawSuggestion =
            serde_json::from_str(r#"{"name": "x", "replaceValue": "y", "_internal": {"z": 1}}"#)
                .expect("valid suggestion JSON");
        let suggestion = normalize_suggestion(raw);
        assert_eq!(suggestion.replace_value.as_deref(), Some("y"));
        assert!(suggestion.internal.is_some());
    }

    /// `specs/mamba.json`'s `activate` subcommand has a generator carrying
    /// `scriptTimeout`, a real field `Generator` does not name.
    #[test]
    fn an_unnamed_field_survives_in_extra() {
        let spec = load("mamba", &[]).expect("mamba is a committed spec");
        let activate = spec
            .subcommands
            .get("activate")
            .expect("mamba has an activate subcommand");
        let generator = &activate.args[0].generators[0];
        assert!(
            generator.extra.contains_key("scriptTimeout"),
            "scriptTimeout is not a named field and must still survive"
        );
    }

    /// `specs/fisher.json`'s `install/IlanCosman/tide@v5` subcommand carries
    /// `description`, `displayName`, `icon` and `priority` together;
    /// `specs/yalc.json`'s `installations` sets `requiresSubcommand`;
    /// `specs/bw.json`'s `delete` sets `isDangerous`; `specs/pnpm.json`'s
    /// root sets `filterStrategy`.
    #[test]
    fn subcommand_simple_fields_are_typed() {
        let fisher = load("fisher", &[]).expect("fisher is a committed spec");
        let tide = fisher
            .subcommands
            .get("install")
            .expect("fisher has an install subcommand")
            .subcommands
            .get("IlanCosman/tide@v5")
            .expect("fisher install has an IlanCosman/tide@v5 subcommand");
        assert!(tide.description.is_some());
        assert_eq!(tide.display_name.as_deref(), Some("Tide"));
        assert!(tide.icon.is_some());
        assert_eq!(tide.priority, Some(99));

        let yalc = load("yalc", &[]).expect("yalc is a committed spec");
        let installations = yalc
            .subcommands
            .get("installations")
            .expect("yalc has an installations subcommand");
        assert_eq!(installations.requires_subcommand, Some(true));

        let bw = load("bw", &[]).expect("bw is a committed spec");
        let delete = bw
            .subcommands
            .get("delete")
            .expect("bw has a delete subcommand");
        assert_eq!(delete.is_dangerous, Some(true));

        let pnpm = load("pnpm", &[]).expect("pnpm is a committed spec");
        assert_eq!(pnpm.filter_strategy.as_deref(), Some("fuzzy"));
    }

    /// `specs/esbuild.json`'s `--charset` option carries `displayName`,
    /// `icon` and `insertValue` together.
    #[test]
    fn option_simple_fields_are_typed() {
        let spec = load("esbuild", &[]).expect("esbuild is a committed spec");
        let charset = spec
            .options
            .get("--charset")
            .expect("esbuild has --charset");
        assert_eq!(charset.display_name.as_deref(), Some("--charset=utf8"));
        assert!(charset.icon.is_some());
        assert_eq!(charset.insert_value.as_deref(), Some("--charset=utf8"));
    }

    /// `specs/who.json`'s `am` subcommand declares one `additionalSuggestions`
    /// entry, normalized the same way `Arg.suggestions` is; its `icon`,
    /// `priority` and `description` come from `specs/kubectx.json`'s root,
    /// which shows the same fields on a [`Suggestion`] reached that way.
    #[test]
    fn additional_suggestions_normalize_the_same_way_as_other_suggestion_lists() {
        let who = load("who", &[]).expect("who is a committed spec");
        let am = who.subcommands.get("am").expect("who has an am subcommand");
        assert_eq!(am.additional_suggestions.len(), 1);
        assert_eq!(am.additional_suggestions[0].name, vec!["am I".to_string()]);

        let kubectx = load("kubectx", &[]).expect("kubectx is a committed spec");
        let dash = kubectx
            .additional_suggestions
            .iter()
            .find(|s| s.name == vec!["-".to_string()])
            .expect("kubectx has a - additionalSuggestion");
        assert!(dash.icon.is_some());
        assert_eq!(dash.priority, Some(85));
        assert!(dash.description.is_some());
    }

    /// One real example apiece: `specs/airflow.json`'s `roles create role`
    /// argument (`isOptional`, `isVariadic`, `optionsCanBreakVariadicArg`),
    /// `specs/do.json`'s root argument (`isCommand`), `specs/ts-node.json`'s
    /// `script` argument (`isScript`), `specs/gem.json`'s `install GEMNAME`
    /// argument (`debounce`), and `specs/tfsec.json`'s `--out outputFile`
    /// argument (`suggestCurrentToken`).
    #[test]
    fn arg_simple_fields_are_typed() {
        let airflow = load("airflow", &[]).expect("airflow is a committed spec");
        let role = airflow
            .subcommands
            .get("roles")
            .and_then(|roles| roles.subcommands.get("create"))
            .expect("airflow has a roles create subcommand")
            .args
            .iter()
            .find(|arg| arg.name == vec!["role".to_string()])
            .expect("roles create has a role argument");
        assert_eq!(role.is_optional, Some(true));
        assert_eq!(role.is_variadic, Some(true));
        assert_eq!(role.options_can_break_variadic_arg, Some(true));

        let do_spec = load("do", &[]).expect("do is a committed spec");
        assert_eq!(do_spec.args[0].is_command, Some(true));

        let ts_node = load("ts-node", &[]).expect("ts-node is a committed spec");
        assert_eq!(ts_node.args[0].is_script, Some(true));

        let gem = load("gem", &[]).expect("gem is a committed spec");
        let install = gem
            .subcommands
            .get("install")
            .expect("gem has an install subcommand");
        assert_eq!(install.args[0].debounce, Some(true));

        let tfsec = load("tfsec", &[]).expect("tfsec is a committed spec");
        let out = tfsec
            .options
            .get("--out")
            .or_else(|| tfsec.options.get("-O"))
            .expect("tfsec has --out/-O");
        assert_eq!(out.args[0].suggest_current_token, Some(true));
    }

    /// `specs/python.json`'s `python -m` argument carries `isModule` as the
    /// string prefix, not a bare flag.
    #[test]
    fn arg_is_module_carries_the_prefix_string() {
        let spec = load("python", &[]).expect("python is a committed spec");
        let module_flag = spec.options.get("-m").expect("python has -m");
        assert_eq!(module_flag.args[0].is_module.as_deref(), Some("python/"));
    }

    /// `specs/ansible-lint.json`'s `-q` is `isRepeatable: 2`,
    /// `specs/sqlmesh.json`'s `plan --restate-model` is `isRepeatable: true`,
    /// and `specs/adb.json`'s `-s` declares no `isRepeatable` at all.
    #[test]
    fn is_repeatable_models_all_three_shapes() {
        let ansible_lint = load("ansible-lint", &[]).expect("ansible-lint is a committed spec");
        let q = ansible_lint.options.get("-q").expect("ansible-lint has -q");
        assert_eq!(q.is_repeatable, Repeatable::Times(2));

        let sqlmesh = load("sqlmesh", &[]).expect("sqlmesh is a committed spec");
        let restate = sqlmesh
            .subcommands
            .get("plan")
            .expect("sqlmesh has a plan subcommand")
            .options
            .get("--restate-model")
            .expect("plan has --restate-model");
        assert_eq!(restate.is_repeatable, Repeatable::Unlimited);

        let adb = load("adb", &[]).expect("adb is a committed spec");
        let serial = adb.options.get("-s").expect("adb has -s");
        assert_eq!(serial.is_repeatable, Repeatable::Once);
    }

    /// `specs/eza.json`'s `--color-scale`/`--colour-scale` sets an explicit
    /// separator; `specs/ua.json`'s `attach --attach-config` sets the bare
    /// flag. `false` never appears in the corpus, so it is not tested here.
    #[test]
    fn requires_separator_models_the_flag_and_the_explicit_string() {
        let eza = load("eza", &[]).expect("eza is a committed spec");
        let color_scale = eza
            .options
            .get("--color-scale")
            .expect("eza has --color-scale");
        assert_eq!(
            color_scale.requires_separator,
            Some(Separator::Explicit("=".to_string()))
        );

        let ua = load("ua", &[]).expect("ua is a committed spec");
        let attach_config = ua
            .subcommands
            .get("attach")
            .expect("ua has an attach subcommand")
            .options
            .get("--attach-config")
            .expect("attach has --attach-config");
        assert_eq!(attach_config.requires_separator, Some(Separator::Default));
    }

    /// `specs/bw.json`'s `login --method` sets `exclusiveOn`, and
    /// `send create --hidden` sets `dependsOn`.
    #[test]
    fn exclusive_on_and_depends_on_are_kept() {
        let bw = load("bw", &[]).expect("bw is a committed spec");
        let method = bw
            .subcommands
            .get("login")
            .expect("bw has a login subcommand")
            .options
            .get("--method")
            .expect("login has --method");
        assert_eq!(
            method.exclusive_on,
            vec![
                "--sso".to_string(),
                "--apikey".to_string(),
                "--check".to_string()
            ]
        );

        let hidden = bw
            .subcommands
            .get("send")
            .and_then(|send| send.subcommands.get("create"))
            .expect("bw has a send create subcommand")
            .options
            .get("--hidden")
            .expect("create has --hidden");
        assert_eq!(hidden.depends_on, vec!["--text".to_string()]);
    }

    /// Phase 0's budget is 10 ms p95 per spec, and `gcloud/compute.json` at
    /// 6 MB is called out as the worst case in `plans/phase-2-spec-runtime.md`'s
    /// Risks section. This is not an assertion the gate can fail on: the
    /// number is reported and phase 0 §2's binary format is the answer if
    /// it ever needs one.
    #[test]
    fn timing_for_git_and_gcloud_compute() {
        let git_start = Instant::now();
        load("git", &[]).expect("git is a committed spec");
        let git_elapsed = git_start.elapsed();

        let gcloud_start = Instant::now();
        load("gcloud/compute", &[]).expect("gcloud/compute is a committed spec");
        let gcloud_elapsed = gcloud_start.elapsed();

        eprintln!("git: {git_elapsed:?}, gcloud/compute: {gcloud_elapsed:?}");
    }

    /// A regression guard on `spec_keys()` itself: if it stops finding the
    /// full corpus, every other test in this file silently checks less than
    /// it claims to.
    #[test]
    fn spec_keys_finds_the_whole_corpus() {
        assert_eq!(spec_keys().len(), 1481);
    }
}
