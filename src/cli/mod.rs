//! The `tinytex` command line: a thin layer over the library (design note,
//! section 5.1).
//!
//! Every flag that describes a compile sets one field of [`CompileRequest`] or
//! [`Config`]; output is the library's serialized types wrapped with a
//! `schema` field. [`run`] takes its environment through [`Host`] and its
//! streams through [`Io`], so tests drive it in process with a mocked
//! [`Exec`]; the binary passes [`Host::system`] and [`Io::stdio`].

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::log::{AccessibilityWarnings, InstallFailure, LatexError, SearchTerm};
use crate::tlmgr::{PathAction, UpdateOpts};
use crate::{
    CompileError, CompileRequest, Compiled, Compiler, Config, Distribution, Event, Exec,
    IndexEngine, Observer, StdExec, Tlmgr, TlmgrError, UpdateOutcome, detect, log,
};

mod args;

pub use args::{Cli, Command, CompileArgs, Global, TlmgrOp};
use args::{EventsFormat, PathArg};

/// The JSON schema version every output object carries.
pub const SCHEMA: u32 = 1;

type EnvFn = Arc<dyn Fn(&str) -> Option<OsString> + Send + Sync>;
type DetectFn = Arc<dyn Fn(&Config) -> Option<Distribution> + Send + Sync>;

/// What the command reads from its surroundings, injectable for tests.
pub struct Host {
    /// The process environment.
    pub env: EnvFn,
    /// [`detect::find`].
    pub detect: DetectFn,
    /// [`Distribution::is_writable`].
    pub writable: Arc<dyn Fn(&Distribution) -> bool + Send + Sync>,
    /// The `Exec` for one command, given its whole-command deadline.
    pub exec: Arc<dyn Fn(Option<Duration>) -> Arc<dyn Exec> + Send + Sync>,
    /// All of standard input.
    pub stdin: Arc<dyn Fn() -> io::Result<String> + Send + Sync>,
}

impl Host {
    /// The real environment, process execution and standard input.
    pub fn system() -> Self {
        Self {
            env: Arc::new(|name| std::env::var_os(name)),
            detect: Arc::new(detect::find),
            writable: Arc::new(Distribution::is_writable),
            exec: Arc::new(|deadline| match deadline {
                Some(budget) => Arc::new(StdExec::with_deadline(budget)),
                None => Arc::new(StdExec::new()),
            }),
            stdin: Arc::new(|| {
                let mut text = String::new();
                io::stdin().read_to_string(&mut text)?;
                Ok(text)
            }),
        }
    }
}

/// The two output streams.
pub struct Io {
    out: Mutex<Box<dyn Write + Send>>,
    err: Mutex<Box<dyn Write + Send>>,
}

impl Io {
    /// Streams over any two writers.
    pub fn new(out: impl Write + Send + 'static, err: impl Write + Send + 'static) -> Self {
        Self {
            out: Mutex::new(Box::new(out)),
            err: Mutex::new(Box::new(err)),
        }
    }

    /// The process's standard output and standard error.
    pub fn stdio() -> Self {
        Self::new(io::stdout(), io::stderr())
    }

    fn out(&self, text: &str) {
        let mut w = self.out.lock().unwrap_or_else(|e| e.into_inner());
        let _ = w.write_all(text.as_bytes());
        let _ = w.flush();
    }

    fn err(&self, text: &str) {
        let mut w = self.err.lock().unwrap_or_else(|e| e.into_inner());
        let _ = w.write_all(text.as_bytes());
        let _ = w.flush();
    }
}

/// A failure with its stable `code` and exit code.
struct Failure {
    code: &'static str,
    exit: i32,
    message: String,
    details: Option<Value>,
    /// Extra lines for the plain-text form.
    notes: Vec<String>,
}

impl Failure {
    fn new(code: &'static str, exit: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            exit,
            message: message.into(),
            details: None,
            notes: Vec::new(),
        }
    }

    fn io(e: &io::Error, what: &str) -> Self {
        Self::new("io", 3, format!("{what}: {e}"))
    }

    fn no_installation() -> Self {
        Self::new("no_tex_installation", 3, "no TeX installation found")
    }

    fn from_compile(e: &CompileError) -> Self {
        let (code, exit) = compile_error_status(e);
        let mut f = Self::new(code, exit, e.to_string());
        f.details = serde_json::to_value(e)
            .ok()
            .and_then(|mut v| v.get_mut("details").map(Value::take))
            .filter(|d| !d.is_null());
        if let CompileError::Latex { hints, log, .. } = e {
            for h in hints {
                f.notes.push(format!("hint: {}", hint_text(*h)));
            }
            if let Some(log) = log {
                f.notes.push(format!("log: {}", log.display()));
            }
        }
        f
    }

    fn from_tlmgr(e: &TlmgrError) -> Self {
        let (code, exit) = tlmgr_error_status(e);
        Self::new(code, exit, e.to_string())
    }
}

fn hint_text(h: log::Hint) -> &'static str {
    match h {
        log::Hint::UnsupportedUnicode => {
            "a Unicode character the input encoding cannot typeset; try the xelatex engine"
        }
        log::Hint::InlineMath => "some text needs math delimiters ($...$)",
        log::Hint::Ghostscript => "the engine could not start Ghostscript; is it installed?",
        log::Hint::GhostscriptMisconfigured => {
            "Ghostscript cannot find its initialization file; reinstall it"
        }
        log::Hint::LuaotfloadFallback => "a luaotfload font-fallback crash; update the fonts",
    }
}

/// The CLI `code` and exit code of a [`CompileError`] (design note, section 5.1).
pub fn compile_error_status(e: &CompileError) -> (&'static str, i32) {
    match e {
        CompileError::NoTexInstallation => ("no_tex_installation", 3),
        CompileError::EngineNotFound { .. } => ("engine_not_found", 3),
        CompileError::Latex { .. } => ("latex_error", 1),
        CompileError::PackageInstall {
            reason: InstallFailure::TexLiveTooOld,
            ..
        } => ("texlive_too_old", 5),
        CompileError::PackageInstall { .. } => ("package_install", 1),
        CompileError::Bibliography { .. } => ("bibliography", 1),
        CompileError::Index { .. } => ("index", 1),
        CompileError::NoOutput => ("no_output", 1),
        CompileError::InvalidRequest { .. } => ("invalid_request", 2),
        CompileError::Cancelled => ("cancelled", 4),
        // The `--timeout` deadline surfaces as a timed-out spawn.
        CompileError::Io(e) if e.kind() == io::ErrorKind::TimedOut => ("cancelled", 4),
        CompileError::Io(_) => ("io", 3),
    }
}

/// The CLI `code` and exit code of a [`TlmgrError`].
pub fn tlmgr_error_status(e: &TlmgrError) -> (&'static str, i32) {
    match e {
        TlmgrError::TexLiveTooOld { .. } => ("texlive_too_old", 5),
        TlmgrError::Interrupted => ("cancelled", 4),
        TlmgrError::Io(e) if e.kind() == io::ErrorKind::TimedOut => ("cancelled", 4),
        _ => ("tlmgr", 1),
    }
}

// ---------- JSON output ----------

/// `value` as a JSON object with `schema` added (a non-object is wrapped as
/// `{"value": ...}`).
fn with_schema(value: Value) -> Value {
    match value {
        Value::Object(mut m) => {
            m.insert("schema".into(), SCHEMA.into());
            Value::Object(m)
        }
        other => json!({ "schema": SCHEMA, "value": other }),
    }
}

fn lossy(p: &mut PathBuf) {
    if p.to_str().is_none() {
        *p = PathBuf::from(p.to_string_lossy().into_owned());
    }
}

fn lossy_opt(p: &mut Option<PathBuf>) {
    if let Some(p) = p {
        lossy(p);
    }
}

fn lossy_request(r: &mut CompileRequest) {
    lossy_opt(&mut r.output_dir);
    lossy_opt(&mut r.pdf_file);
    r.input_paths.iter_mut().for_each(lossy);
    let t = &mut r.tools;
    for p in [
        &mut t.engine,
        &mut t.bibtex,
        &mut t.biber,
        &mut t.makeindex,
        &mut t.latexmk,
        &mut t.tectonic,
    ] {
        lossy_opt(p);
    }
}

fn lossy_config(c: &mut Config) {
    lossy_opt(&mut c.install_dir);
    lossy_opt(&mut c.tlmgr_path);
}

fn lossy_dist(d: &mut Distribution) {
    lossy_opt(&mut d.root);
    lossy(&mut d.bin_dir);
    lossy(&mut d.tlmgr);
}

fn to_value<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

// ---------- entry point ----------

struct Ctx<'a> {
    host: &'a Host,
    io: &'a Io,
    json: bool,
}

impl Ctx<'_> {
    fn print_json(&self, v: Value) {
        self.io
            .out(&(serde_json::to_string_pretty(&v).unwrap_or_default() + "\n"));
    }

    fn result(&self, value: Value, plain: impl FnOnce() -> String) {
        if self.json {
            self.print_json(with_schema(value));
        } else {
            let text = plain();
            if !text.is_empty() {
                self.io.out(&(text.trim_end().to_owned() + "\n"));
            }
        }
    }

    fn fail(&self, f: &Failure) -> i32 {
        if self.json {
            let mut v = json!({ "code": f.code, "message": f.message });
            if let Some(d) = &f.details {
                v["details"] = d.clone();
            }
            self.print_json(with_schema(v));
        } else {
            let mut text = format!("error: {}\n", f.message);
            for n in &f.notes {
                text.push_str(n);
                text.push('\n');
            }
            self.io.err(&text);
        }
        f.exit
    }
}

struct Printer<'a> {
    io: &'a Io,
    json: bool,
}

impl Observer for Printer<'_> {
    fn event(&self, e: &Event) {
        if self.json {
            let mut v = to_value(e);
            if let Value::Object(m) = &mut v {
                if let Some(tag) = m.remove("event") {
                    m.insert("type".into(), tag);
                }
                m.insert("schema".into(), SCHEMA.into());
            }
            self.io
                .err(&(serde_json::to_string(&v).unwrap_or_default() + "\n"));
        } else {
            self.io.err(&format!("{e}\n"));
        }
    }
}

/// Runs the command line `args` (the first is the program name) and returns
/// the exit code: 0 success, 1 a LaTeX, package, bibliography, index, no-output
/// or `tlmgr` failure, 2 a usage error or invalid request, 3 an environment
/// problem, 4 cancelled or timed out, 5 TeX Live older than the repository.
pub fn run<I, T>(args: I, host: &Host, io: &Io) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            let text = e.render().to_string();
            return if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) {
                io.out(&text);
                0
            } else {
                io.err(&text);
                2
            };
        }
    };
    let ctx = Ctx {
        host,
        io,
        json: cli.global.json,
    };
    match dispatch(&ctx, &cli) {
        Ok(code) => code,
        Err(f) => ctx.fail(&f),
    }
}

/// The clap `Command`, for tools that walk the definition.
pub fn command() -> clap::Command {
    Cli::command()
}

fn dispatch(ctx: &Ctx<'_>, cli: &Cli) -> Result<i32, Failure> {
    let g = &cli.global;
    match &cli.command {
        Command::Compile(a) => compile(ctx, g, a),
        Command::Detect => detect_cmd(ctx, g),
        Command::Tlmgr { op } => tlmgr(ctx, g, op),
        Command::Log { file } => log_cmd(ctx, file),
    }
}

fn base_config(host: &Host, g: &Global) -> Config {
    let mut c = Config::from_env_with(|n| (host.env)(n));
    apply_config(&mut c, g);
    c
}

fn apply_config(c: &mut Config, g: &Global) {
    if let Some(d) = &g.install_dir {
        c.install_dir = Some(d.clone());
    }
    if let Some(r) = &g.repository {
        c.repository = Some(r.clone());
    }
    if !g.tlmgr_opt.is_empty() {
        c.tlmgr_opts = g.tlmgr_opt.clone();
    }
    if let Some(u) = g.tlmgr_update {
        c.tlmgr_update = u.into();
    }
    if let Some(p) = g.prefer {
        c.prefer = p.into();
    }
    if let Some(t) = &g.tlmgr_path {
        c.tlmgr_path = Some(t.clone());
    }
    c.usermode |= g.usermode;
    c.delete_tlpdb |= g.delete_tlpdb;
    c.verbose |= g.verbose;
}

fn invalid(message: String) -> Failure {
    Failure::new("invalid_request", 2, message)
}

/// The `{request, config}` the flags produce, and the distribution found.
fn build_pair(
    ctx: &Ctx<'_>,
    g: &Global,
    a: &CompileArgs,
) -> Result<(CompileRequest, Config, Option<Distribution>), Failure> {
    let mut request = CompileRequest::default();
    let mut config = None;
    let from_file = a.request.is_some();
    if let Some(source) = &a.request {
        let text = if source == "-" {
            (ctx.host.stdin)().map_err(|e| Failure::io(&e, "could not read standard input"))?
        } else {
            std::fs::read_to_string(source)
                .map_err(|e| Failure::io(&e, &format!("could not read {source}")))?
        };
        let value: Value = serde_json::from_str(&text)
            .map_err(|e| invalid(format!("the request in {source} is not valid JSON: {e}")))?;
        let (req_value, cfg_value) = match &value {
            Value::Object(m) if m.contains_key("request") => {
                (m["request"].clone(), m.get("config").cloned())
            }
            _ => (value.clone(), None),
        };
        request = serde_json::from_value(req_value)
            .map_err(|e| invalid(format!("the request in {source} is not a request: {e}")))?;
        if let Some(c) = cfg_value {
            config =
                Some(serde_json::from_value::<Config>(c).map_err(|e| {
                    invalid(format!("the config in {source} is not a config: {e}"))
                })?);
        }
    }
    let mut config = config.unwrap_or_else(|| base_config(ctx.host, g));
    apply_config(&mut config, g);
    apply_request(&mut request, a)?;

    let dist = (ctx.host.detect)(&config);
    if a.auto_install {
        request.auto_install = true;
    } else if a.no_auto_install {
        request.auto_install = false;
    } else if !from_file {
        request.auto_install = dist.as_ref().is_some_and(|d| (ctx.host.writable)(d));
    }
    Ok((request, config, dist))
}

fn apply_request(r: &mut CompileRequest, a: &CompileArgs) -> Result<(), Failure> {
    if let Some(e) = a.pdf_engine {
        r.engine = e.into();
    }
    if !a.pdf_engine_opt.is_empty() {
        r.engine_args = a.pdf_engine_opt.clone();
    }
    if let Some(b) = a.bib_engine {
        r.bib = b.into();
    }
    if a.index_engine.is_some() || !a.index_engine_opt.is_empty() {
        let old = r.index.take();
        let command = a
            .index_engine
            .clone()
            .or_else(|| old.as_ref().map(|i| i.command.clone()))
            .unwrap_or_else(|| "makeindex".to_owned());
        let args = if a.index_engine_opt.is_empty() {
            old.map(|i| i.args).unwrap_or_default()
        } else {
            a.index_engine_opt.clone()
        };
        r.index = Some(IndexEngine { command, args });
    }
    if let Some(n) = a.min {
        r.min_runs = n;
    }
    if let Some(n) = a.max {
        r.max_runs = n;
    }
    if let Some(d) = a.driver {
        r.driver = d.into();
    }
    if let Some(v) = a.recover {
        r.recover = v.into();
    }
    if let Some(v) = a.on_bib_failure {
        r.on_bib_failure = v.into();
    }
    if let Some(v) = a.keep_log {
        r.keep_log = v.into();
    }
    if let Some(v) = a.output {
        r.output = v.into();
    }
    if let Some(d) = &a.output_dir {
        r.output_dir = Some(d.clone());
    }
    if let Some(p) = &a.pdf_file {
        r.pdf_file = Some(p.clone());
    }
    if !a.input_path.is_empty() {
        r.input_paths = a.input_path.clone();
    }
    if a.no_clean {
        r.clean = crate::Clean::Never;
    } else if let Some(c) = a.clean {
        r.clean = c.into();
    }
    for (name, path) in &a.tool {
        let slot = match name.as_str() {
            "engine" => &mut r.tools.engine,
            "bibtex" => &mut r.tools.bibtex,
            "biber" => &mut r.tools.biber,
            "makeindex" => &mut r.tools.makeindex,
            "latexmk" => &mut r.tools.latexmk,
            "tectonic" => &mut r.tools.tectonic,
            other => return Err(invalid(format!("unknown tool `{other}`"))),
        };
        *slot = Some(path.clone());
    }
    Ok(())
}

fn compile(ctx: &Ctx<'_>, g: &Global, a: &CompileArgs) -> Result<i32, Failure> {
    let (mut request, mut config, dist) = build_pair(ctx, g, a)?;
    if a.print_request {
        lossy_request(&mut request);
        lossy_config(&mut config);
        let v = json!({ "schema": SCHEMA, "request": request, "config": config });
        ctx.print_json(v);
        return Ok(0);
    }
    let x = (ctx.host.exec)(g.timeout);
    let printer = Printer {
        io: ctx.io,
        json: matches!(g.events, EventsFormat::Json),
    };
    let compiled = Compiler::new(dist, config)
        .compile(&a.file, &request, &*x, &printer)
        .map_err(|e| Failure::from_compile(&e))?;
    let mut compiled = compiled;
    report_compiled(ctx, &mut compiled);
    Ok(0)
}

fn report_compiled(ctx: &Ctx<'_>, c: &mut Compiled) {
    lossy(&mut c.pdf);
    lossy_opt(&mut c.log);
    let plain = || c.pdf.display().to_string();
    if !ctx.json {
        for w in &c.diagnostics.warnings {
            ctx.io.err(&format!("warning: {w}\n"));
        }
        if c.diagnostics.max_runs_reached {
            ctx.io
                .err("warning: the run limit stopped a rerun the document still wanted\n");
        }
        if let Some(b) = &c.diagnostics.bibliography_failed {
            ctx.io
                .err(&format!("warning: the bibliography tool failed: {b}\n"));
        }
        for n in &c.diagnostics.notices {
            ctx.io.err(&format!("note: {n}\n"));
        }
    }
    ctx.result(to_value(&*c), plain);
}

fn detect_cmd(ctx: &Ctx<'_>, g: &Global) -> Result<i32, Failure> {
    let config = base_config(ctx.host, g);
    let mut dist = (ctx.host.detect)(&config).ok_or_else(Failure::no_installation)?;
    lossy_dist(&mut dist);
    let plain = || {
        let mut t = format!(
            "kind: {:?}\nbin_dir: {}\n",
            dist.kind,
            dist.bin_dir.display()
        );
        if let Some(root) = &dist.root {
            t.push_str(&format!("root: {}\n", root.display()));
        }
        t
    };
    ctx.result(to_value(&dist), plain);
    Ok(0)
}

fn tlmgr(ctx: &Ctx<'_>, g: &Global, op: &TlmgrOp) -> Result<i32, Failure> {
    let config = base_config(ctx.host, g);
    let dist = (ctx.host.detect)(&config).ok_or_else(Failure::no_installation)?;
    let x = (ctx.host.exec)(g.timeout);
    let printer = Printer {
        io: ctx.io,
        json: matches!(g.events, EventsFormat::Json),
    };
    let t = Tlmgr::new(&dist, &config, &*x, &printer);
    let tl = |e: TlmgrError| Failure::from_tlmgr(&e);
    let ok = json!({ "ok": true });
    match op {
        TlmgrOp::Version => {
            let v = t.version().map_err(tl)?;
            ctx.result(to_value(&v), || format!("TeX Live {} ({})", v.year, v.date));
        }
        TlmgrOp::Search { term } => {
            let term = if term.starts_with('/') {
                term.clone()
            } else {
                format!("/{term}")
            };
            let pkgs = t.search_file(&SearchTerm::File(term)).map_err(tl)?;
            ctx.result(json!({ "packages": pkgs }), || pkgs.join("\n"));
        }
        TlmgrOp::Info {
            pkgs,
            fields,
            only_installed,
        } => {
            let fields: Vec<&str> = fields.split(',').collect();
            let rows = t.info(pkgs, &fields, *only_installed).map_err(tl)?;
            ctx.result(json!({ "fields": fields, "rows": rows }), || {
                rows.iter()
                    .map(|r| r.join(","))
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        }
        TlmgrOp::Install { pkgs } => {
            let o = t.install(pkgs).map_err(tl)?;
            ctx.result(to_value(&o), || {
                format!(
                    "installed: {}\nalready present: {}\nfailed: {}",
                    o.installed.join(" "),
                    o.already_present.join(" "),
                    o.failed.join(" ")
                )
            });
            if !o.failed.is_empty() {
                return Ok(1);
            }
        }
        TlmgrOp::Update { self_only } => {
            let opts = UpdateOpts {
                self_only: *self_only,
                delete_tlpdb: config.delete_tlpdb,
                ..UpdateOpts::default()
            };
            let o = t.update(opts).map_err(tl)?;
            ctx.result(to_value(&o), || format!("{o:?}"));
            if matches!(o, UpdateOutcome::TexLiveTooOld { .. }) {
                return Ok(5);
            }
        }
        TlmgrOp::Remove { pkgs } => {
            t.remove(pkgs).map_err(tl)?;
            ctx.result(ok, String::new);
        }
        TlmgrOp::Repository { url } => match url {
            Some(url) => {
                t.set_repository(url).map_err(tl)?;
                ctx.result(ok, String::new);
            }
            None => {
                let r = t.repository().map_err(tl)?;
                ctx.result(json!({ "repository": r }), || r.clone());
            }
        },
        TlmgrOp::Path { action } => {
            t.path(match action {
                PathArg::Add => PathAction::Add,
                PathArg::Remove => PathAction::Remove,
            })
            .map_err(tl)?;
            ctx.result(ok, String::new);
        }
        TlmgrOp::Fmtutil => match t.fmtutil() {
            Ok(()) => ctx.result(ok, String::new),
            Err(w) => {
                ctx.result(json!({ "ok": true, "warning": w }), String::new);
                ctx.io.err(&format!("warning: {w}\n"));
            }
        },
        TlmgrOp::Run {
            changes_installation,
            args,
        } => {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = t.run(&refs, *changes_installation).map_err(tl)?;
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            let status = out.status.code();
            if ctx.json {
                ctx.print_json(with_schema(
                    json!({ "status": status, "stdout": stdout, "stderr": stderr }),
                ));
            } else {
                ctx.io.out(&stdout);
                ctx.io.err(&stderr);
            }
            if !out.status.success() {
                return Ok(1);
            }
        }
    }
    Ok(0)
}

/// The diagnostics of one engine log, as `tinytex log` prints them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogReport {
    /// [`log::latex_error`].
    pub error: Option<LatexError>,
    /// [`log::missing_packages`].
    pub missing_packages: Vec<SearchTerm>,
    /// [`log::missing_modules`].
    pub missing_modules: Vec<String>,
    /// [`log::missing_hyphenation`].
    pub missing_hyphenation: Vec<String>,
    /// [`log::latex_warnings`].
    pub warnings: Vec<String>,
    /// [`log::needs_rerun`].
    pub needs_rerun: bool,
    /// [`log::index_error`].
    pub index_error: Option<String>,
    /// [`log::accessibility`].
    pub accessibility: AccessibilityWarnings,
}

impl LogReport {
    /// The report for `text`.
    pub fn of(text: &str) -> Self {
        Self {
            error: log::latex_error(text, None),
            missing_packages: log::missing_packages(text, None),
            missing_modules: log::missing_modules(text),
            missing_hyphenation: log::missing_hyphenation(text),
            warnings: log::latex_warnings(text),
            needs_rerun: log::needs_rerun(text),
            index_error: log::index_error(text),
            accessibility: log::accessibility(text),
        }
    }

    fn plain(&self) -> String {
        let mut t = String::new();
        if let Some(e) = &self.error {
            t.push_str(&format!("error:\n{}\n", e.summary));
            for h in &e.hints {
                t.push_str(&format!("hint: {}\n", hint_text(*h)));
            }
        }
        for p in &self.missing_packages {
            t.push_str(&format!("missing: {p}\n"));
        }
        for m in self.missing_modules.iter().chain(&self.missing_hyphenation) {
            t.push_str(&format!("missing package: {m}\n"));
        }
        for w in &self.warnings {
            t.push_str(&format!("warning: {w}\n"));
        }
        if self.needs_rerun {
            t.push_str("the document needs another run\n");
        }
        if let Some(e) = &self.index_error {
            t.push_str(&format!("index error: {e}\n"));
        }
        if t.is_empty() {
            t.push_str("no problems found\n");
        }
        t
    }
}

fn log_cmd(ctx: &Ctx<'_>, file: &std::path::Path) -> Result<i32, Failure> {
    let bytes = std::fs::read(file)
        .map_err(|e| Failure::io(&e, &format!("could not read {}", file.display())))?;
    let report = LogReport::of(&String::from_utf8_lossy(&bytes));
    ctx.result(to_value(&report), || report.plain());
    Ok(0)
}
