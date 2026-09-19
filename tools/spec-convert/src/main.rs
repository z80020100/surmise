//! Freeze the static half of a completion-spec corpus.
//!
//! A spec is an ES module whose default export is built by running code, so
//! reading one means running it. This tool runs the corpus once, keeps every
//! field that is data and drops every field that is a function, and writes one
//! JSON file per spec. surmise compiles that JSON in and never runs JavaScript.
//!
//! An argument whose suggestions only a function could have produced is marked
//! `"dyn": true` and listed in `dynamic.txt`. A native Rust generator answers
//! those, the way the Git readers already do.
//!
//! The output is deterministic. Keys are sorted and array order is the spec's
//! own, so the same corpus gives the same bytes and a regeneration diffs to
//! exactly what changed upstream.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use boa_engine::builtins::promise::PromiseState;
use boa_engine::module::{Module, SimpleModuleLoader};
use boa_engine::object::builtins::JsPromise;
use boa_engine::{Context, JsObject, JsValue, Source, js_string};
use serde_json::{Map, Value, json};

/// Every field of a spec that is data rather than code. A key that does not
/// apply to a node kind is simply absent from that node.
const DATA_KEYS: &[&str] = &[
    "debounce",
    "default",
    "definition",
    "deprecated",
    "dependsOn",
    "description",
    "displayName",
    "exclusiveOn",
    "filterStrategy",
    "hidden",
    "icon",
    "insertValue",
    "isCommand",
    "isDangerous",
    "isModule",
    "isOptional",
    "isPersistent",
    "isRepeatable",
    "isRequired",
    "isScript",
    "isVariadic",
    "name",
    "optionsCanBreakVariadicArg",
    "priority",
    "requiresEquals",
    "requiresSeparator",
    "requiresSubcommand",
    "separator",
    "suggestCurrentToken",
    "template",
    "type",
];

/// A generator's data fields. `script` is here because a spec often writes it
/// as a command line rather than as code, and that form survives conversion.
const GENERATOR_DATA_KEYS: &[&str] = &[
    "filterStrategy",
    "getQueryTerm",
    "script",
    "scriptTimeout",
    "splitOn",
    "template",
    "trigger",
];

/// Why an argument lost its suggestions.
const REASON_CUSTOM: &str = "custom";
const REASON_SCRIPT_FN: &str = "script-fn";
const REASON_POST_PROCESS: &str = "postProcess";
const REASON_LOAD_SPEC_FN: &str = "loadSpec-fn";

struct Args {
    corpus: PathBuf,
    out: PathBuf,
    exclude: Vec<String>,
    jobs: usize,
}

fn usage() -> ! {
    eprintln!(
        "usage: spec-convert --corpus <package-dir> --out <dir> [--exclude a,b] [--jobs N]

  --corpus   the unpacked corpus package. It holds `build/` and `LICENSE`
  --out      where the JSON goes. Existing files are overwritten
  --exclude  top-level command names to skip, comma separated
  --jobs     worker threads. Defaults to the machine's parallelism"
    );
    std::process::exit(2)
}

fn parse_args() -> Args {
    let mut corpus = None;
    let mut out = None;
    let mut exclude = Vec::new();
    let mut jobs = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut argv = std::env::args().skip(1);
    while let Some(flag) = argv.next() {
        let mut value = || argv.next().unwrap_or_else(|| usage());
        match flag.as_str() {
            "--corpus" => corpus = Some(PathBuf::from(value())),
            "--out" => out = Some(PathBuf::from(value())),
            "--exclude" => {
                exclude = value()
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            }
            "--jobs" => jobs = value().parse().unwrap_or_else(|_| usage()),
            _ => usage(),
        }
    }
    let (Some(corpus), Some(out)) = (corpus, out) else {
        usage()
    };
    Args {
        corpus,
        out,
        exclude,
        jobs: jobs.max(1),
    }
}

/// What one spec file produced.
struct Outcome {
    /// Path under `build/`, without the extension. This is the spec's identity
    /// and `loadSpec` names it.
    id: String,
    status: Status,
    /// `id`, the token path to the argument's owner and the argument's index,
    /// with the reason it needs code.
    dynamic: Vec<String>,
    counts: Counts,
}

enum Status {
    Written,
    /// The module evaluated and exported no default. It is something a spec
    /// imports rather than a spec, such as `deno/generators.js`.
    NotASpec,
    /// The default export is a function of a version. It resolved to the path
    /// of another spec in the corpus, which converts on its own.
    Versioned(String),
    Failed(String),
}

#[derive(Default, Clone, Copy)]
struct Counts {
    subcommands: usize,
    options: usize,
    args: usize,
    dynamic: usize,
}

impl Counts {
    fn add(&mut self, other: Counts) {
        self.subcommands += other.subcommands;
        self.options += other.options;
        self.args += other.args;
        self.dynamic += other.dynamic;
    }
}

fn main() {
    let args = parse_args();
    let build = args.corpus.join("build");
    if !build.is_dir() {
        eprintln!("spec-convert: {} is not a directory", build.display());
        std::process::exit(1);
    }

    let index = read_corpus_index(&args.corpus);
    let specs = collect_specs(&build, &args.exclude);
    if specs.is_empty() {
        eprintln!("spec-convert: no spec files under {}", build.display());
        std::process::exit(1);
    }
    println!(
        "spec-convert: {} spec files, {} workers",
        specs.len(),
        args.jobs
    );

    let next = AtomicUsize::new(0);
    let outcomes = Mutex::new(Vec::with_capacity(specs.len()));
    std::thread::scope(|scope| {
        for _ in 0..args.jobs {
            scope.spawn(|| {
                // A Boa context is not `Send`, so each worker builds its own.
                // The loader is per context too, which also keeps one spec's
                // module graph out of the next one's.
                let Ok(loader) = SimpleModuleLoader::new(&build) else {
                    return;
                };
                let loader = Rc::new(loader);
                let Ok(mut ctx) = Context::builder().module_loader(loader.clone()).build() else {
                    return;
                };
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = specs.get(i) else { return };
                    let outcome = convert(path, &build, &args.out, &loader, &mut ctx);
                    outcomes.lock().unwrap().push(outcome);
                }
            });
        }
    });

    let mut outcomes = outcomes.into_inner().unwrap();
    outcomes.sort_by(|a, b| a.id.cmp(&b.id));
    write_index(&args, &index, &outcomes);
    write_dynamic(&args.out, &outcomes);
    copy_licence(&args.corpus, &args.out);
    report(&outcomes);
}

/// `build/index.json` is already JSON. It names the top-level commands and the
/// six whose spec is a function of the tool's version.
fn read_corpus_index(corpus: &Path) -> Value {
    std::fs::read(corpus.join("build").join("index.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}))
}

fn collect_specs(build: &Path, exclude: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![build.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // `dynamic/` is the corpus's own lazy loader rather than a spec.
                if path.file_name().is_some_and(|n| n == "dynamic") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "js") {
                // `build/index.js` is the name index and `index.json` says the
                // same thing without being evaluated.
                if path.parent() == Some(build) && path.file_name().is_some_and(|n| n == "index.js")
                {
                    continue;
                }
                out.push(path);
            }
        }
    }
    out.retain(|p| {
        let id = spec_id(p, build);
        !exclude
            .iter()
            .any(|x| id == *x || id.starts_with(&format!("{x}/")))
    });
    out.sort();
    out
}

/// The path under `build/` without the extension. `build/aws/s3.js` is `aws/s3`
/// and that is the string a `loadSpec` names.
fn spec_id(path: &Path, build: &Path) -> String {
    path.strip_prefix(build)
        .unwrap_or(path)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/")
}

fn convert(
    path: &Path,
    build: &Path,
    out_root: &Path,
    loader: &Rc<SimpleModuleLoader>,
    ctx: &mut Context,
) -> Outcome {
    let id = spec_id(path, build);
    let fail = |reason: String| Outcome {
        id: id.clone(),
        status: Status::Failed(reason),
        dynamic: Vec::new(),
        counts: Counts::default(),
    };

    let default = match evaluate(path, loader, ctx) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };

    let Some(object) = default.as_object() else {
        return Outcome {
            id,
            status: Status::NotASpec,
            dynamic: Vec::new(),
            counts: Counts::default(),
        };
    };

    // Six specs export a function of the tool's version. Calling it with no
    // version asks for the newest, and the answer is the path of another spec
    // in this corpus, which this run converts on its own.
    if object.is_callable() {
        return match versioned_target(&object, ctx) {
            Ok(target) => {
                let body = json!({ "name": command_name(&id), "versionedSpecPath": target });
                match write_json(out_root, &id, &body) {
                    Ok(()) => Outcome {
                        id,
                        status: Status::Versioned(target),
                        dynamic: Vec::new(),
                        counts: Counts::default(),
                    },
                    Err(e) => fail(e),
                }
            }
            Err(e) => fail(e),
        };
    }

    let mut walk = Walk {
        ctx,
        id: &id,
        trail: Vec::new(),
        dynamic: Vec::new(),
        counts: Counts::default(),
    };
    let body = walk.object(&object);
    let (dynamic, counts) = (walk.dynamic, walk.counts);
    match write_json(out_root, &id, &body) {
        Ok(()) => Outcome {
            id,
            status: Status::Written,
            dynamic,
            counts,
        },
        Err(e) => fail(e),
    }
}

fn evaluate(
    path: &Path,
    loader: &Rc<SimpleModuleLoader>,
    ctx: &mut Context,
) -> Result<JsValue, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open: {e}"))?;
    let source = Source::from_reader(file, Some(path));
    let module = Module::parse(source, None, ctx).map_err(|e| format!("parse: {e}"))?;
    loader.insert(
        path.canonicalize().unwrap_or_else(|_| path.to_path_buf()),
        module.clone(),
    );
    let promise = module.load_link_evaluate(ctx);
    ctx.run_jobs().map_err(|e| format!("jobs: {e}"))?;
    match promise.state() {
        PromiseState::Fulfilled(_) => {}
        PromiseState::Rejected(e) => return Err(format!("eval: {}", e.display())),
        PromiseState::Pending => return Err("eval: still pending".into()),
    }
    module
        .namespace(ctx)
        .get(js_string!("default"), ctx)
        .map_err(|e| format!("default: {e}"))
}

/// Call a versioned spec's default export with no version and read the path it
/// answers with. The corpus resolves that to the newest one it holds.
fn versioned_target(function: &JsObject, ctx: &mut Context) -> Result<String, String> {
    let returned = function
        .call(&JsValue::undefined(), &[], ctx)
        .map_err(|e| format!("versioned call: {e}"))?;
    let resolved = match returned.as_object().map(JsPromise::from_object) {
        Some(Ok(promise)) => {
            ctx.run_jobs().map_err(|e| format!("jobs: {e}"))?;
            match promise.state() {
                PromiseState::Fulfilled(v) => v,
                PromiseState::Rejected(e) => {
                    return Err(format!("versioned: {}", e.display()));
                }
                PromiseState::Pending => return Err("versioned: still pending".into()),
            }
        }
        _ => returned,
    };
    let object = resolved
        .as_object()
        .ok_or_else(|| format!("versioned answer is {}", resolved.type_of()))?;
    let path = object
        .get(js_string!("versionedSpecPath"), ctx)
        .map_err(|e| format!("versionedSpecPath: {e}"))?;
    path.as_string()
        .map(|s| s.to_std_string_escaped())
        .ok_or_else(|| "versionedSpecPath is not a string".into())
}

/// The name a person types to reach a spec. A directory keeps its own part in
/// `index`, so `fig/index` is the command `fig`, and a scoped package keeps the
/// slash its scope needs.
fn command_name(id: &str) -> &str {
    let id = id.strip_suffix("/index").unwrap_or(id);
    if id.starts_with('@') {
        id
    } else {
        id.rsplit('/').next().unwrap_or(id)
    }
}

/// The walk that turns one evaluated spec into JSON. It carries the token path
/// so a dynamic argument can name where it lives.
struct Walk<'a, 'b> {
    ctx: &'a mut Context,
    id: &'b str,
    trail: Vec<String>,
    dynamic: Vec<String>,
    counts: Counts,
}

impl Walk<'_, '_> {
    fn get(&mut self, object: &JsObject, key: &str) -> Option<JsValue> {
        let value = object
            .get(js_string!(key.to_owned()), &mut *self.ctx)
            .ok()?;
        (!value.is_undefined() && !value.is_null()).then_some(value)
    }

    /// A node. Data keys first, then the three child lists, then the fields
    /// that need a rule of their own.
    fn object(&mut self, object: &JsObject) -> Value {
        let mut out = Map::new();
        for key in DATA_KEYS {
            if let Some(value) = self.get(object, key)
                && let Some(json) = self.plain(&value)
            {
                out.insert((*key).to_owned(), json);
            }
        }

        for (key, counter) in [
            ("subcommands", 0usize),
            ("options", 1),
            ("additionalSuggestions", 2),
        ] {
            let Some(value) = self.get(object, key) else {
                continue;
            };
            let items = self.list(&value);
            if items.is_empty() {
                continue;
            }
            let mut converted = Vec::with_capacity(items.len());
            for item in &items {
                let Some(child) = item.as_object() else {
                    if let Some(json) = self.plain(item) {
                        converted.push(json);
                    }
                    continue;
                };
                match counter {
                    0 => self.counts.subcommands += 1,
                    1 => self.counts.options += 1,
                    _ => {}
                }
                // A name can be a string or a list of aliases. The first one is
                // the path a person would have typed to get here.
                let name = self
                    .get(&child, "name")
                    .and_then(|v| self.first_name(&v))
                    .unwrap_or_else(|| "?".to_owned());
                self.trail.push(name);
                converted.push(Value::Object(self.child(&child)));
                self.trail.pop();
            }
            out.insert(key.to_owned(), Value::Array(converted));
        }

        if let Some(value) = self.get(object, "args") {
            let items = self.list(&value);
            let mut converted = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                let Some(arg) = item.as_object() else {
                    continue;
                };
                self.counts.args += 1;
                converted.push(Value::Object(self.arg(&arg, index)));
            }
            if !converted.is_empty() {
                // An `args` written as a bare object stays a list here. One
                // shape downstream is one code path downstream.
                out.insert("args".to_owned(), Value::Array(converted));
            }
        }

        if let Some(value) = self.get(object, "suggestions")
            && let Some(json) = self.plain(&value)
        {
            out.insert("suggestions".to_owned(), json);
        }

        // A string `loadSpec` names another file in the corpus and survives.
        // A function one builds a spec at completion time and cannot.
        if let Some(value) = self.get(object, "loadSpec") {
            if let Some(name) = value.as_string() {
                out.insert(
                    "loadSpec".to_owned(),
                    Value::String(name.to_std_string_escaped()),
                );
            } else if value.as_object().is_some_and(|o| o.is_callable()) {
                out.insert("dyn".to_owned(), Value::Bool(true));
                self.note(usize::MAX, REASON_LOAD_SPEC_FN);
            }
        }

        for key in ["parserDirectives", "cache"] {
            if let Some(value) = self.get(object, key)
                && let Some(json) = self.plain(&value)
                && !json.as_object().is_some_and(Map::is_empty)
            {
                out.insert(key.to_owned(), json);
            }
        }

        Value::Object(out)
    }

    fn child(&mut self, object: &JsObject) -> Map<String, Value> {
        match self.object(object) {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    /// An argument, plus the generators that say where its suggestions come
    /// from. A generator that only a function could answer marks the argument.
    fn arg(&mut self, object: &JsObject, index: usize) -> Map<String, Value> {
        let mut out = self.child(object);
        let mut reasons = Vec::new();
        if let Some(value) = self.get(object, "generators") {
            let items = self.list(&value);
            let mut converted = Vec::with_capacity(items.len());
            for item in &items {
                let Some(generator) = item.as_object() else {
                    continue;
                };
                converted.push(Value::Object(self.generator(&generator, &mut reasons)));
            }
            if !converted.is_empty() {
                out.insert("generators".to_owned(), Value::Array(converted));
            }
        }
        if !reasons.is_empty() {
            out.insert("dyn".to_owned(), Value::Bool(true));
            self.counts.dynamic += 1;
            reasons.sort_unstable();
            reasons.dedup();
            self.note(index, &reasons.join(","));
        }
        out
    }

    fn generator(
        &mut self,
        object: &JsObject,
        reasons: &mut Vec<&'static str>,
    ) -> Map<String, Value> {
        let mut out = Map::new();
        for key in GENERATOR_DATA_KEYS {
            if let Some(value) = self.get(object, key)
                && let Some(json) = self.plain(&value)
            {
                out.insert((*key).to_owned(), json);
            }
        }
        for key in ["suggestions", "cache"] {
            if let Some(value) = self.get(object, key)
                && let Some(json) = self.plain(&value)
            {
                out.insert(key.to_owned(), json);
            }
        }

        // `custom` builds the whole list in code. Nothing survives it.
        if self.is_function(object, "custom") {
            reasons.push(REASON_CUSTOM);
        }
        // A `script` written as a command line converts. Written as a function
        // it does not, and then the generator has no command to run.
        if self.is_function(object, "script") {
            reasons.push(REASON_SCRIPT_FN);
        } else if out.contains_key("script") && self.is_function(object, "postProcess") {
            // The command survives and the parse of its output does not.
            reasons.push(REASON_POST_PROCESS);
        }
        out
    }

    fn is_function(&mut self, object: &JsObject, key: &str) -> bool {
        self.get(object, key)
            .and_then(|v| v.as_object().map(|o| o.is_callable()))
            .unwrap_or(false)
    }

    /// One line of `dynamic.txt`: the spec, the tokens that reach the owner,
    /// the argument's index and why it needs code.
    fn note(&mut self, index: usize, reason: &str) {
        let where_ = if index == usize::MAX {
            "loadSpec".to_owned()
        } else {
            index.to_string()
        };
        self.dynamic.push(format!(
            "{}\t{}\t{}\t{}",
            self.id,
            self.trail.join(" "),
            where_,
            reason
        ));
    }

    fn first_name(&mut self, value: &JsValue) -> Option<String> {
        if let Some(name) = value.as_string() {
            return Some(name.to_std_string_escaped());
        }
        self.list(value)
            .first()
            .and_then(|v| v.as_string().map(|s| s.to_std_string_escaped()))
    }

    /// A field that holds one item or several. Both shapes are common and the
    /// walk treats them the same.
    fn list(&mut self, value: &JsValue) -> Vec<JsValue> {
        let Some(object) = value.as_object() else {
            return vec![value.clone()];
        };
        if !object.is_array() {
            return vec![value.clone()];
        }
        let length = object
            .get(js_string!("length"), &mut *self.ctx)
            .ok()
            .and_then(|v| v.as_number())
            .unwrap_or(0.0) as usize;
        (0..length)
            .filter_map(|i| object.get(i as u32, &mut *self.ctx).ok())
            .filter(|v| !v.is_undefined() && !v.is_null())
            .collect()
    }

    /// Anything with no rule of its own. A function becomes nothing, so the
    /// caller drops the key rather than writing a null it would have to read.
    fn plain(&mut self, value: &JsValue) -> Option<Value> {
        if let Some(text) = value.as_string() {
            return Some(Value::String(text.to_std_string_escaped()));
        }
        if let Some(flag) = value.as_boolean() {
            return Some(Value::Bool(flag));
        }
        if let Some(number) = value.as_number() {
            // Every JavaScript number is a float and almost none of these are.
            // 4116 of the 4162 in the corpus are whole, so write them whole:
            // `"priority": 100` is what a reader expects to see in a diff.
            if number.fract() == 0.0 && number.abs() <= i64::MAX as f64 {
                return Some(Value::Number((number as i64).into()));
            }
            return serde_json::Number::from_f64(number).map(Value::Number);
        }
        let object = value.as_object()?;
        if object.is_callable() {
            return None;
        }
        if object.is_array() {
            let items = self.list(value);
            return Some(Value::Array(
                items.iter().filter_map(|v| self.plain(v)).collect(),
            ));
        }
        let keys = object.own_property_keys(&mut *self.ctx).ok()?;
        let mut out = Map::new();
        for key in keys {
            let name = key.to_string();
            let Ok(field) = object.get(key, &mut *self.ctx) else {
                continue;
            };
            if field.is_undefined() || field.is_null() {
                continue;
            }
            if let Some(json) = self.plain(&field) {
                out.insert(name, json);
            }
        }
        Some(Value::Object(out))
    }
}

fn write_json(out_root: &Path, id: &str, body: &Value) -> Result<(), String> {
    let path = out_root.join(format!("{id}.json"));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }
    // Pretty rather than compact. The point of committing this is that a
    // regeneration diffs line by line, and the build compresses it anyway.
    let mut text = serde_json::to_string_pretty(body).map_err(|e| format!("encode: {e}"))?;
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))
}

fn write_index(args: &Args, corpus_index: &Value, outcomes: &[Outcome]) {
    let written: Vec<&str> = outcomes
        .iter()
        .filter(|o| matches!(o.status, Status::Written | Status::Versioned(_)))
        .map(|o| o.id.as_str())
        .collect();
    let keep = |name: &&Value| {
        name.as_str().is_some_and(|n| {
            !args
                .exclude
                .iter()
                .any(|x| n == x || n.starts_with(&format!("{x}/")))
        })
    };
    let take = |key: &str| -> Vec<Value> {
        corpus_index
            .get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter(keep).cloned().collect())
            .unwrap_or_default()
    };
    let body = json!({
        "source": corpus_source(&args.corpus),
        "commands": top_level(&written),
        "completions": take("completions"),
        "diffVersioned": take("diffVersionedCompletions"),
        "specs": written,
    });
    if let Err(e) = write_json(&args.out, "index", &body) {
        eprintln!("spec-convert: {e}");
    }
}

/// Where this data came from, taken from the corpus's own manifest. The
/// attribution in `THIRD_PARTY.md` names the same package.
/// The names a person can type. A spec that sits directly under the corpus
/// root is one, and so is the `index` of a directory there. Everything deeper
/// is a file that a `loadSpec` reaches rather than a command of its own.
fn top_level(specs: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = specs
        .iter()
        .map(|id| id.strip_suffix("/index").unwrap_or(id))
        .filter(|id| {
            // A scoped package keeps the slash that its scope needs.
            let depth = id.matches('/').count();
            depth == 0 || (id.starts_with('@') && depth == 1)
        })
        .map(str::to_owned)
        .collect();
    out.sort();
    out.dedup();
    out
}

fn corpus_source(corpus: &Path) -> Value {
    let manifest: Value = std::fs::read(corpus.join("package.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let field = |key: &str| manifest.get(key).cloned().unwrap_or(Value::Null);
    json!({ "package": field("name"), "version": field("version") })
}

/// The backlog. One line per argument a native Rust generator would have to
/// answer, sorted so the file diffs cleanly.
fn write_dynamic(out: &Path, outcomes: &[Outcome]) {
    let mut lines: Vec<&str> = outcomes
        .iter()
        .flat_map(|o| o.dynamic.iter().map(String::as_str))
        .collect();
    lines.sort_unstable();
    let mut text = String::from(
        "# Arguments whose suggestions only code could produce. A native Rust\n\
         # generator answers one of these or it stays empty.\n\
         # spec<TAB>tokens<TAB>arg index<TAB>reason\n",
    );
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    if let Err(e) = std::fs::write(out.join("dynamic.txt"), text) {
        eprintln!("spec-convert: dynamic.txt: {e}");
    }
}

/// The corpus is MIT and the converted data carries its descriptions, so the
/// licence travels with it.
fn copy_licence(corpus: &Path, out: &Path) {
    match std::fs::copy(corpus.join("LICENSE"), out.join("LICENSE")) {
        Ok(_) => {}
        Err(e) => eprintln!("spec-convert: LICENSE: {e}"),
    }
}

fn report(outcomes: &[Outcome]) {
    let mut totals = Counts::default();
    let mut written = 0usize;
    let mut skipped = 0usize;
    let mut versioned = Vec::new();
    let mut failed = Vec::new();
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    for outcome in outcomes {
        totals.add(outcome.counts);
        match &outcome.status {
            Status::Written => written += 1,
            Status::NotASpec => skipped += 1,
            Status::Versioned(target) => versioned.push((&outcome.id, target)),
            Status::Failed(why) => failed.push((&outcome.id, why)),
        }
        for line in &outcome.dynamic {
            if let Some(reason) = line.rsplit('\t').next() {
                for one in reason.split(',') {
                    *reasons.entry(leak(one)).or_default() += 1;
                }
            }
        }
    }
    println!(
        "\n  written     {written}\n  versioned   {}\n  not a spec  {skipped}\n  failed      {}",
        versioned.len(),
        failed.len()
    );
    println!(
        "\n  subcommands {}\n  options     {}\n  arguments   {}\n  dynamic     {} ({:.1}% of arguments)",
        totals.subcommands,
        totals.options,
        totals.args,
        totals.dynamic,
        100.0 * totals.dynamic as f64 / totals.args.max(1) as f64
    );
    if !reasons.is_empty() {
        println!("\n  why they are dynamic");
        for (reason, count) in &reasons {
            println!("    {reason:<14} {count}");
        }
    }
    for (id, target) in &versioned {
        println!("\n  versioned   {id} -> {target}");
    }
    for (id, why) in &failed {
        println!("  FAILED      {id}: {why}");
    }
    if !failed.is_empty() {
        std::process::exit(1);
    }
}

/// The reason strings are compile-time constants joined with a comma. Splitting
/// them back apart borrows from a temporary, and the report needs them to
/// outlive it. There are four.
fn leak(reason: &str) -> &'static str {
    for known in [
        REASON_CUSTOM,
        REASON_SCRIPT_FN,
        REASON_POST_PROCESS,
        REASON_LOAD_SPEC_FN,
    ] {
        if reason == known {
            return known;
        }
    }
    "other"
}
