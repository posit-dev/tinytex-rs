//! [`Tlmgr`]: the `tlmgr` wrapper, its result and error types, and the
//! install policy of design section 3.2.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;
use std::process::Output;
use std::sync::{Arc, LazyLock};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

use crate::config::Config;
use regex::Regex;

use crate::ErrorCode;
pub use crate::detect::TlVersion;
use crate::detect::{Distribution, parse_tl_version};
use crate::exec::{CommandSpec, Event, Exec, Observer};
use crate::lock::{Entry, InstallGuard};
use crate::log::{SearchTerm, missing_packages};

/// What [`Tlmgr::install`] did, per package.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct InstallOutcome {
    /// Missing before, present after.
    pub installed: Vec<String>,
    /// Present before the call (or by the time the lock was taken).
    pub already_present: Vec<String>,
    /// Still missing after the retry.
    pub failed: Vec<String>,
    /// Non-fatal `fmtutil` failures from format rebuilds the install ran.
    pub warnings: Vec<FmtutilWarning>,
}

/// What [`Tlmgr::update`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
#[non_exhaustive]
pub enum UpdateOutcome {
    /// `tlmgr update` ran; `warning` is a non-fatal `fmtutil` failure.
    Updated { warning: Option<FmtutilWarning> },
    /// The local TeX Live is older than the remote repository; no update ran
    /// and every later one in this process is skipped.
    TexLiveTooOld { local: u32, remote: u32 },
    /// No command ran: `TlmgrUpdate::Never`, or an update already ran in this
    /// process for this distribution.
    Skipped,
}

/// Options for [`Tlmgr::update`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct UpdateOpts {
    /// Update every installed package (`--all`).
    pub all: bool,
    /// Update `tlmgr` itself only (`update --self`); `all` is then ignored.
    /// Without it, `all` adds `--self` too, as R does (not in user mode).
    pub self_only: bool,
    /// Rebuild formats after the update (unless TeX Live is too old).
    pub run_fmtutil: bool,
    /// Delete the `texlive.tlpdb.<hash>` files under `tlpkg` afterwards.
    pub delete_tlpdb: bool,
    pub more_args: Vec<String>,
}

impl Default for UpdateOpts {
    fn default() -> Self {
        Self {
            all: true,
            self_only: false,
            run_fmtutil: true,
            delete_tlpdb: false,
            more_args: Vec::new(),
        }
    }
}

/// [`Tlmgr::path`] direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathAction {
    Add,
    Remove,
}

/// A failed format rebuild. Non-fatal by construction: the package install
/// already succeeded by the time `fmtutil` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FmtutilWarning {
    pub message: String,
    pub exit_code: Option<i32>,
}

impl std::fmt::Display for FmtutilWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Why a `tlmgr` operation failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TlmgrError {
    #[error("could not run a command: {0}")]
    Io(#[from] io::Error),
    #[error("`{program} {}` failed ({})", args.join(" "), match code { Some(c) => format!("exit status {c}"), None => "no exit status".to_owned() })]
    Command {
        program: String,
        args: Vec<String>,
        code: Option<i32>,
        stderr: String,
    },
    #[error("local TeX Live ({local}) is older than the remote repository ({remote})")]
    TexLiveTooOld { local: u32, remote: u32 },
    #[error("interrupted")]
    Interrupted,
    #[error("could not parse the output of `tlmgr {what}`")]
    Unparseable { what: String, output: String },
}

impl TlmgrError {
    /// The stable code of this error.
    pub fn code(&self) -> ErrorCode {
        ErrorCode::new(match self {
            TlmgrError::Io(_) => "tlmgr.io",
            TlmgrError::Command { .. } => "tlmgr.command_failed",
            TlmgrError::TexLiveTooOld { .. } => "tlmgr.texlive_too_old",
            TlmgrError::Interrupted => "tlmgr.interrupted",
            TlmgrError::Unparseable { .. } => "tlmgr.unparseable",
        })
    }
}

impl Serialize for TlmgrError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("TlmgrError", 2)?;
        s.serialize_field("code", &self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

/// The `tlmgr` wrapper of one distribution.
pub struct Tlmgr<'a> {
    pub(crate) dist: &'a Distribution,
    pub(crate) cfg: &'a Config,
    pub(crate) x: &'a dyn Exec,
    pub(crate) o: &'a dyn Observer,
    pub(crate) entry: Arc<Entry>,
    pub(crate) windows: bool,
}

impl<'a> Tlmgr<'a> {
    pub fn new(
        dist: &'a Distribution,
        cfg: &'a Config,
        x: &'a dyn Exec,
        o: &'a dyn Observer,
    ) -> Self {
        Self {
            dist,
            cfg,
            x,
            o,
            entry: crate::lock::entry_for(dist),
            windows: cfg!(windows),
        }
    }

    /// Builds the commands in their Windows forms on any platform (tests).
    #[doc(hidden)]
    pub fn for_windows(mut self, windows: bool) -> Self {
        self.windows = windows;
        self
    }
}

/// Whether two distributions share one registry entry (one lock, one set of
/// update flags). For tests.
#[doc(hidden)]
pub fn share_lock(a: &Distribution, b: &Distribution) -> bool {
    Arc::ptr_eq(&crate::lock::entry_for(a), &crate::lock::entry_for(b))
}

/// `tlmgr update` reports this when the local TeX Live is a release behind
/// the remote repository (R `check_tl_version`).
static TOO_OLD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Local TeX Live \((\d+)\) is older than remote repository \((\d+)\)")
        .expect("valid regex")
});

/// The lines `tlmgr info` prints for a package it does not know: one naming the package
/// (`package X not locally!` with `--only-installed`, `package X not found neither locally
/// nor remote!` without it), then two lines closing the run.
static UNKNOWN_PACKAGE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^tlmgr: (?:package \S+ not (?:locally|found neither locally nor remote)!|action info returned an error; continuing\.|An error has occurred\. See above messages\. Exiting\.)$",
    )
    .expect("valid regex")
});

/// Whether `stderr` from a failed `tlmgr info` says only that packages are unknown: it names
/// at least one and holds no other complaint.
fn only_unknown_packages(stderr: &str) -> bool {
    let mut lines = stderr.lines().map(str::trim).filter(|l| !l.is_empty());
    let mut named = false;
    for line in lines.by_ref() {
        if !UNKNOWN_PACKAGE_LINE.is_match(line) {
            return false;
        }
        named |= line.starts_with("tlmgr: package ");
    }
    named
}

/// A `texlive.tlpdb.<md5>` copy under `tlpkg` (R `delete_tlpdb_files`).
static HASHED_TLPDB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^texlive[.]tlpdb.*[.][0-9a-f]{32}$").expect("valid regex"));

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

fn arg_strings(spec: &CommandSpec) -> Vec<String> {
    spec.args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

fn dedup(pkgs: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    pkgs.iter()
        .filter(|p| seen.insert((*p).clone()))
        .cloned()
        .collect()
}

/// Quarto 1's message (`fmtutilFailureMessage`, `texlive.ts` at `v1.12.3`).
fn fmtutil_warning(code: Option<i32>, stderr: &str, cause: Option<&io::Error>) -> FmtutilWarning {
    let what = match (code, cause) {
        (Some(c), _) => format!("exited {c}"),
        (None, Some(e)) => format!("could not run: {e}"),
        (None, None) => "was terminated by a signal".to_owned(),
    };
    let stderr = stderr.trim();
    let detail = if stderr.is_empty() {
        String::new()
    } else {
        format!("\n{stderr}")
    };
    FmtutilWarning {
        message: format!(
            "Failed to rebuild format tree (`fmtutil-sys --all` {what}). This is non-fatal \u{2014} package installation will continue.{detail}"
        ),
        exit_code: code,
    }
}

/// Single-quotes a string for `sh`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Escapes a string for an AppleScript string literal.
fn applescript_escape(s: &str) -> String {
    s.replace('\\', r"\\").replace('"', "\\\"")
}

impl Tlmgr<'_> {
    // ---------- plumbing ----------

    fn lock(&self) -> Result<InstallGuard<'_>, TlmgrError> {
        self.entry
            .acquire(self.x)
            .map_err(|_| TlmgrError::Interrupted)
    }

    fn program(&self, name: &str) -> CommandSpec {
        let path = std::env::var_os("PATH");
        self.dist.command_for(name, self.windows, path.as_deref())
    }

    /// `tlmgr` with the configured global options in front of `args`.
    fn tlmgr_spec(&self, args: &[String], changes_installation: bool) -> CommandSpec {
        let mut spec = self.program("tlmgr");
        if self.cfg.usermode {
            spec.args.push("--usermode".into());
        }
        spec.args.extend(self.cfg.tlmgr_opts.iter().map(Into::into));
        spec.args.extend(args.iter().map(Into::into));
        spec.changes_installation = changes_installation;
        spec
    }

    /// Runs one command under the cancellation rule: after any `Err` or
    /// non-zero exit, and after any command that changes the installation,
    /// `interrupted()` is tested first and wins.
    fn exec(&self, spec: &CommandSpec) -> Result<Output, TlmgrError> {
        let result = self.x.run(spec);
        let failed = !matches!(&result, Ok(out) if out.status.success());
        if (failed || spec.changes_installation) && self.x.interrupted() {
            return Err(TlmgrError::Interrupted);
        }
        result.map_err(TlmgrError::Io)
    }

    fn command_error(spec: &CommandSpec, out: &Output) -> TlmgrError {
        TlmgrError::Command {
            program: spec
                .program
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            args: arg_strings(spec),
            code: out.status.code(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }
    }

    /// Runs `tlmgr args` and turns a non-zero exit into an error.
    fn checked(&self, args: &[String], changes_installation: bool) -> Result<Output, TlmgrError> {
        let spec = self.tlmgr_spec(args, changes_installation);
        let out = self.exec(&spec)?;
        if out.status.success() {
            Ok(out)
        } else {
            Err(Self::command_error(&spec, &out))
        }
    }

    fn emit(&self, e: Event) {
        self.o.event(&e);
    }

    // ---------- read-only ----------

    /// `tlmgr --version`: the TeX Live year and the `tlmgr` date.
    pub fn version(&self) -> Result<TlVersion, TlmgrError> {
        let out = self.checked(&strings(&["--version"]), false)?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        parse_tl_version(&text).ok_or(TlmgrError::Unparseable {
            what: "--version".to_owned(),
            output: text,
        })
    }

    /// The packages that contain a file matching `term`, in `tlmgr`'s order.
    ///
    /// A [`SearchTerm::Package`] names its package already and runs nothing.
    pub fn search_file(&self, term: &SearchTerm) -> Result<Vec<String>, TlmgrError> {
        self.emit(Event::Searching { term: term.clone() });
        let file = match term {
            SearchTerm::Package(name) => return Ok(vec![name.clone()]),
            SearchTerm::File(file) => file,
        };
        let out = self.checked(&strings(&["search", "--file", "--global", file]), false)?;
        let text = String::from_utf8_lossy(&out.stdout);
        // The term is a regular expression anchored at the end of the path
        // (R: `grep(paste0('/', x, '$'), l)`, so `/mf` does not match `mfplain`).
        let matcher = Regex::new(&format!("(?:{file})$")).ok();
        let hits = |line: &str| match &matcher {
            Some(re) => re.is_match(line),
            None => line.ends_with(file.as_str()),
        };
        let mut found: Vec<String> = Vec::new();
        let mut current: Option<String> = None;
        for line in text.lines() {
            let is_package =
                !line.starts_with(char::is_whitespace) && line.ends_with(':') && line.len() > 1;
            if is_package {
                // `metafont.x86_64-darwin`: the part after the dot is the platform.
                let name = line.trim_end_matches(':');
                current = Some(name.split('.').next().unwrap_or(name).to_owned());
            } else if let Some(pkg) = &current
                && hits(line.trim_end())
            {
                if !found.contains(pkg) {
                    found.push(pkg.clone());
                }
                current = None;
            }
        }
        Ok(found)
    }

    /// `tlmgr info --list`: the requested `fields` of each package, one row per
    /// output line. An unknown package does not fail the call; its row is absent.
    pub fn info(
        &self,
        pkgs: &[String],
        fields: &[&str],
        only_installed: bool,
    ) -> Result<Vec<Vec<String>>, TlmgrError> {
        let mut args = strings(&["info", "--list"]);
        if only_installed {
            args.push("--only-installed".to_owned());
        }
        args.push("--data".to_owned());
        args.push(fields.join(","));
        args.extend(pkgs.iter().cloned());
        let spec = self.tlmgr_spec(&args, false);
        let out = self.exec(&spec)?;
        // R reads the list whatever the status is: `tlmgr info` exits non-zero
        // when one of several packages is unknown, and still lists the rest.
        // When none of them is known it lists nothing, so an empty list with a
        // failing status is the answer for unknown packages, not a failure.
        // Anything else on stderr is a real failure and stays an error.
        if !out.status.success()
            && out.stdout.is_empty()
            && !only_unknown_packages(&String::from_utf8_lossy(&out.stderr))
        {
            return Err(Self::command_error(&spec, &out));
        }
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| l.split(',').map(str::to_owned).collect())
            .collect())
    }

    /// The repository `tlmgr` uses (`tlmgr option repository`).
    pub fn repository(&self) -> Result<String, TlmgrError> {
        let out = self.checked(&strings(&["option", "repository"]), false)?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        text.lines()
            .find(|l| l.contains("(repository):"))
            .or_else(|| text.lines().rfind(|l| !l.trim().is_empty()))
            .and_then(|l| l.rsplit_once(": ").map(|(_, url)| url.trim().to_owned()))
            .filter(|url| !url.is_empty())
            .ok_or(TlmgrError::Unparseable {
                what: "option repository".to_owned(),
                output: text,
            })
    }

    /// The installed ones among `pkgs` (R `check_installed`).
    fn installed_among(&self, pkgs: &[String]) -> Result<BTreeSet<String>, TlmgrError> {
        if pkgs.is_empty() {
            return Ok(BTreeSet::new());
        }
        Ok(self
            .info(pkgs, &["name"], true)?
            .into_iter()
            .filter_map(|row| row.into_iter().next())
            .collect())
    }

    /// Whether `pkg` is installed. Any failure to find out counts as no.
    pub fn is_installed(&self, pkg: &str) -> bool {
        self.installed_among(&[pkg.to_owned()])
            .is_ok_and(|set| set.contains(pkg))
    }

    /// The packages a LaTeX log asks for (R `parse_packages`): file searches
    /// run through `tlmgr search`; a search that fails is skipped, so the
    /// caller still sees the other packages. Only an interruption is an error.
    pub fn packages_for_log(
        &self,
        log: &str,
        missfont: Option<&str>,
    ) -> Result<Vec<String>, TlmgrError> {
        let mut pkgs: Vec<String> = Vec::new();
        for term in missing_packages(log, missfont) {
            match self.search_file(&term) {
                Ok(found) => {
                    for p in found {
                        if !pkgs.contains(&p) {
                            pkgs.push(p);
                        }
                    }
                }
                Err(TlmgrError::Interrupted) => return Err(TlmgrError::Interrupted),
                Err(_) => {}
            }
        }
        Ok(pkgs)
    }

    /// [`Tlmgr::packages_for_log`] then [`Tlmgr::install`] (R `parse_install`).
    pub fn install_for_log(
        &self,
        log: &str,
        missfont: Option<&str>,
    ) -> Result<InstallOutcome, TlmgrError> {
        let pkgs = self.packages_for_log(log, missfont)?;
        self.install(&pkgs)
    }

    /// The escape hatch: `tlmgr args` with the caller's classification of
    /// whether the command changes the installation. It takes no lock and does
    /// not check the exit status.
    pub fn run(&self, args: &[&str], changes_installation: bool) -> Result<Output, TlmgrError> {
        self.exec(&self.tlmgr_spec(&strings(args), changes_installation))
    }

    // ---------- install ----------

    /// Installs `pkgs` in one `tlmgr install` call (design section 3.2).
    ///
    /// Under the install lock: packages already present are left alone (and
    /// when all are present nothing runs). When `tlmgr_update` is `Auto` and
    /// some requested package is already installed, `tlmgr update` runs first
    /// (at most once per distribution per process). Exit status 0 or 255 is
    /// accepted; any other status, or a package still missing, runs
    /// `update --self` and installs again once. The caller reruns its engine
    /// after any attempt. `tlmgr path add` is never run. Fails with
    /// [`TlmgrError::TexLiveTooOld`] when packages stay missing and the
    /// distribution is known to be a release behind the repository.
    pub fn install(&self, pkgs: &[String]) -> Result<InstallOutcome, TlmgrError> {
        let pkgs = dedup(pkgs);
        if pkgs.is_empty() {
            return Ok(InstallOutcome::default());
        }
        let guard = self.lock()?;
        self.install_locked(&guard, &pkgs)
    }

    fn install_locked(
        &self,
        guard: &InstallGuard<'_>,
        pkgs: &[String],
    ) -> Result<InstallOutcome, TlmgrError> {
        use crate::config::TlmgrUpdate;

        let mut out = InstallOutcome::default();
        // Under the lock: another thread may have installed these already.
        let before = self.installed_among(pkgs)?;
        let (present, missing): (Vec<String>, Vec<String>) =
            pkgs.iter().cloned().partition(|p| before.contains(p));
        out.already_present = present;
        if missing.is_empty() {
            return Ok(out);
        }

        if self.cfg.tlmgr_update == TlmgrUpdate::Auto && !out.already_present.is_empty() {
            self.update_for_install(guard, &UpdateOpts::default(), true, &mut out)?;
        }

        for (i, name) in missing.iter().enumerate() {
            self.emit(Event::InstallingPackage {
                name: name.clone(),
                index: i + 1,
                of: missing.len(),
            });
        }
        let accepted = self.install_command(pkgs)?;
        let mut now = self.installed_among(pkgs)?;
        self.announce(&before, &now);

        let incomplete = |now: &BTreeSet<String>| pkgs.iter().any(|p| !now.contains(p));
        if !accepted || incomplete(&now) {
            let retry = UpdateOpts {
                all: false,
                self_only: true,
                ..UpdateOpts::default()
            };
            if self.update_for_install(guard, &retry, false, &mut out)? {
                let accepted_again = self.install_command(pkgs)?;
                let _ = accepted_again;
                let after = self.installed_among(pkgs)?;
                self.announce(&now, &after);
                now = after;
            }
        }

        out.installed = missing
            .iter()
            .filter(|p| now.contains(*p))
            .cloned()
            .collect();
        out.failed = missing
            .iter()
            .filter(|p| !now.contains(*p))
            .cloned()
            .collect();
        if !out.failed.is_empty()
            && let Some((local, remote)) = self.entry.flags().too_old
        {
            return Err(TlmgrError::TexLiveTooOld { local, remote });
        }
        Ok(out)
    }

    /// `tlmgr install pkgs`; whether the exit status is one `tlmgr` install
    /// accepts (0, or 255 as Quarto 1 tolerates).
    fn install_command(&self, pkgs: &[String]) -> Result<bool, TlmgrError> {
        let mut args = vec!["install".to_owned()];
        args.extend(pkgs.iter().cloned());
        let out = self.exec(&self.tlmgr_spec(&args, true))?;
        Ok(matches!(out.status.code(), Some(0) | Some(255)))
    }

    fn announce(&self, before: &BTreeSet<String>, after: &BTreeSet<String>) {
        let names: Vec<String> = after.difference(before).cloned().collect();
        if !names.is_empty() {
            self.emit(Event::PackagesInstalled { names });
        }
    }

    /// The update an install runs. Returns whether the install should run
    /// again afterwards: the update ran (even if it failed), not when it was
    /// skipped or TeX Live is too old, where a second install cannot help.
    fn update_for_install(
        &self,
        guard: &InstallGuard<'_>,
        opts: &UpdateOpts,
        once: bool,
        out: &mut InstallOutcome,
    ) -> Result<bool, TlmgrError> {
        match self.update_locked(guard, opts, once) {
            Ok(UpdateOutcome::Updated { warning }) => {
                out.warnings.extend(warning);
                Ok(true)
            }
            Ok(_) => Ok(false),
            Err(TlmgrError::Interrupted) => Err(TlmgrError::Interrupted),
            // R ignores a failed update and installs anyway.
            Err(_) => Ok(true),
        }
    }

    // ---------- update ----------

    /// `tlmgr update`, lazily: skipped with `TlmgrUpdate::Never`, after an
    /// update already ran for this distribution in this process, and once
    /// TeX Live is known to be older than the repository.
    pub fn update(&self, opts: UpdateOpts) -> Result<UpdateOutcome, TlmgrError> {
        let guard = self.lock()?;
        self.update_locked(&guard, &opts, true)
    }

    fn update_locked(
        &self,
        guard: &InstallGuard<'_>,
        opts: &UpdateOpts,
        once: bool,
    ) -> Result<UpdateOutcome, TlmgrError> {
        use crate::config::TlmgrUpdate;

        if self.cfg.tlmgr_update == TlmgrUpdate::Never {
            return Ok(UpdateOutcome::Skipped);
        }
        let flags = self.entry.flags();
        if let Some((local, remote)) = flags.too_old {
            return Ok(UpdateOutcome::TexLiveTooOld { local, remote });
        }
        if once && flags.update_done {
            return Ok(UpdateOutcome::Skipped);
        }

        let mut args = vec!["update".to_owned()];
        let with_self = !self.cfg.usermode;
        if opts.self_only {
            args.push("--self".to_owned());
        } else if opts.all {
            args.push("--all".to_owned());
            if with_self {
                args.push("--self".to_owned());
            }
        }
        args.extend(opts.more_args.iter().cloned());
        if args.iter().any(|a| a == "--self") {
            self.emit(Event::UpdatingTlmgr);
        }
        if args.iter().any(|a| a == "--all") {
            self.emit(Event::UpdatingPackages);
        }

        let spec = self.tlmgr_spec(&args, true);
        let out = self.exec(&spec)?;
        self.entry.update_flags(|f| f.update_done = true);

        let text = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let too_old = TOO_OLD
            .captures(&text)
            .and_then(|c| Some((c[1].parse::<u32>().ok()?, c[2].parse::<u32>().ok()?)));
        let outcome = if let Some((local, remote)) = too_old {
            self.entry
                .update_flags(|f| f.too_old = Some((local, remote)));
            UpdateOutcome::TexLiveTooOld { local, remote }
        } else if !out.status.success() {
            return Err(Self::command_error(&spec, &out));
        } else {
            let warning = if opts.run_fmtutil {
                self.fmtutil_locked(guard)?
            } else {
                None
            };
            UpdateOutcome::Updated { warning }
        };
        if opts.delete_tlpdb || self.cfg.delete_tlpdb {
            self.delete_tlpdb_files();
        }
        Ok(outcome)
    }

    fn delete_tlpdb_files(&self) {
        let Some(root) = &self.dist.root else { return };
        let Ok(entries) = fs::read_dir(root.join("tlpkg")) else {
            return;
        };
        for entry in entries.flatten() {
            if HASHED_TLPDB.is_match(&entry.file_name().to_string_lossy()) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }

    // ---------- fmtutil ----------

    /// Rebuilds the formats (`fmtutil --sys --all`). A failure is a warning,
    /// never an error: the packages are already installed by then.
    pub fn fmtutil(&self) -> Result<(), FmtutilWarning> {
        let interrupted = |what: &str| FmtutilWarning {
            message: format!("Format rebuild {what}: the render was cancelled. This is non-fatal."),
            exit_code: None,
        };
        let guard = self.lock().map_err(|_| interrupted("skipped"))?;
        match self.fmtutil_locked(&guard) {
            Ok(None) => Ok(()),
            Ok(Some(w)) => Err(w),
            Err(_) => Err(interrupted("stopped")),
        }
    }

    fn fmtutil_locked(
        &self,
        _guard: &InstallGuard<'_>,
    ) -> Result<Option<FmtutilWarning>, TlmgrError> {
        let mut spec = self.program("fmtutil");
        if spec.args.is_empty() {
            spec.args
                .push(if self.cfg.usermode { "--user" } else { "--sys" }.into());
            spec.args.push("--all".into());
        }
        spec.changes_installation = true;
        self.emit(Event::RebuildingFormats);
        let result = self.x.run(&spec);
        if self.x.interrupted() {
            return Err(TlmgrError::Interrupted);
        }
        Ok(match result {
            Ok(out) if out.status.success() => None,
            Ok(out) => Some(fmtutil_warning(
                out.status.code(),
                &String::from_utf8_lossy(&out.stderr),
                None,
            )),
            Err(e) => Some(fmtutil_warning(None, "", Some(&e))),
        })
    }

    // ---------- other installation changes ----------

    pub fn remove(&self, pkgs: &[String]) -> Result<(), TlmgrError> {
        let pkgs = dedup(pkgs);
        if pkgs.is_empty() {
            return Ok(());
        }
        let _guard = self.lock()?;
        let mut args = vec!["remove".to_owned()];
        args.extend(pkgs);
        self.checked(&args, true).map(drop)
    }

    /// Adds or removes the TeX Live binaries on the system `PATH`. On macOS,
    /// where `/usr/local/bin` is not writable, this writes
    /// `/etc/paths.d/TinyTeX` instead (the way R does).
    pub fn path(&self, action: PathAction) -> Result<(), TlmgrError> {
        let file = Path::new("/etc/paths.d/TinyTeX");
        let paths_d = cfg!(target_os = "macos") && paths_d_needed();
        self.path_with(action, paths_d.then_some(file))
    }

    #[doc(hidden)]
    pub fn path_with(
        &self,
        action: PathAction,
        paths_d_file: Option<&Path>,
    ) -> Result<(), TlmgrError> {
        let _guard = self.lock()?;
        let Some(file) = paths_d_file else {
            let verb = match action {
                PathAction::Add => "add",
                PathAction::Remove => "remove",
            };
            return self.checked(&strings(&["path", verb]), true).map(drop);
        };
        let file_text = file.to_string_lossy().into_owned();
        let shell = match action {
            PathAction::Add => {
                let dir = self.dist.bin_dir.to_string_lossy().into_owned();
                let current = fs::read_to_string(file).unwrap_or_default();
                if current.lines().collect::<Vec<_>>() == [dir.as_str()] {
                    return Ok(());
                }
                format!("echo {} > {}", sh_quote(&dir), sh_quote(&file_text))
            }
            PathAction::Remove => format!("rm -f {}", sh_quote(&file_text)),
        };
        let mut spec = CommandSpec::new("osascript");
        spec.args = vec![
            "-e".into(),
            format!(
                "do shell script \"{}\" with administrator privileges",
                applescript_escape(&shell)
            )
            .into(),
        ];
        spec.changes_installation = true;
        let out = self.exec(&spec)?;
        if out.status.success() {
            Ok(())
        } else {
            Err(Self::command_error(&spec, &out))
        }
    }

    /// Points `tlmgr` at `url` (`tlmgr option repository`).
    pub fn set_repository(&self, url: &str) -> Result<(), TlmgrError> {
        let _guard = self.lock()?;
        self.checked(&strings(&["option", "repository", url]), true)
            .map(drop)
    }
}

/// Whether `/usr/local/bin` is unwritable, so `path` must use `/etc/paths.d`
/// (R `use_paths_d`). Probes by creating a file, which is all `access(2)`
/// would tell, without `unsafe`.
fn paths_d_needed() -> bool {
    let probe = Path::new("/usr/local/bin/.tinytex-write-probe");
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(probe)
    {
        Ok(_) => {
            let _ = fs::remove_file(probe);
            false
        }
        Err(e) => e.kind() != io::ErrorKind::AlreadyExists,
    }
}
