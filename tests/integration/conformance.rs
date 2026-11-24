//! Conformance of the test fakes to the real `tlmgr`.
//!
//! The fake `tlmgr` of `tlmgr_model` builds its own reply text from a model.
//! These tests compare that reply, and what the crate makes of it, with real
//! output, so a fake cannot drift from the tool again (the first auto-install
//! bug hid behind a fake that answered "not installed" with exit 0).
//!
//! * Step (a): [`real_tlmgr`] runs one `tlmgr` query and returns its status,
//!   stdout, and stderr ([`Captured`]). With `TINYTEX_RS_RECAPTURE_DIR` set,
//!   the `real` tests also save what they ran as fixtures.
//! * Step (b), `offline`: the fake against the captured fixtures. Runs
//!   everywhere.
//! * Step (c), `real`: the fake against the real tool. Read-only `info`
//!   queries run against any TeX that `find` returns; queries that contact a
//!   repository run only inside a sandbox (`sandbox::gate`), because they write
//!   into the tree.
//!
//! What is compared: the status class (zero or not), the stdout rows as a set,
//! and the API outcome of `info`, `is_installed`, `install`, and `search_file`.
//! Not compared: trailer sentences, progress lines, repository URLs, the
//! verification header, the frozen-repository banner, revision numbers, sizes,
//! stderr line order, or exit 255.
//!
//! A case where the fake is known to differ is `#[ignore]`d with the phase that
//! fixes the fake; running it with `--run-ignored all` shows the difference.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Output;

use tinytex::log::SearchTerm;
use tinytex::{Config, Distribution, Exec, NullObserver, StdExec, Tlmgr};

use super::tlmgr_model::{dist, fake, model};

/// One `tlmgr` run: exit status, stdout, and stderr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Captured {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl Captured {
    pub fn from_output(o: &Output) -> Self {
        Self {
            status: o.status.code(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }

    /// A captured fixture, read directly (not through the model's `Reply`).
    pub fn fixture(name: &str) -> Self {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tlmgr");
        let read = |ext: &str| {
            std::fs::read_to_string(dir.join(format!("{name}.{ext}")))
                .unwrap_or_else(|e| panic!("fixture {name}.{ext}: {e}"))
        };
        Self {
            status: Some(read("status").trim().parse().expect("a numeric status")),
            stdout: read("stdout"),
            stderr: read("stderr"),
        }
    }

    /// Zero or not: the class that tests compare, never the exact status.
    pub fn ok(&self) -> bool {
        self.status == Some(0)
    }

    /// The stdout rows as a set, without the `tlmgr: package repository ..`
    /// line that repository calls print.
    pub fn rows(&self) -> BTreeSet<String> {
        self.stdout
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("tlmgr:"))
            .map(str::to_owned)
            .collect()
    }

    /// Writes `NAME.{status,stdout,stderr}` into `dir`, for recapturing a
    /// fixture from a real run.
    pub fn save(&self, dir: &Path, name: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let status = self.status.map_or("none".to_owned(), |c| c.to_string());
        std::fs::write(dir.join(format!("{name}.status")), format!("{status}\n")).unwrap();
        std::fs::write(dir.join(format!("{name}.stdout")), &self.stdout).unwrap();
        std::fs::write(dir.join(format!("{name}.stderr")), &self.stderr).unwrap();
    }
}

/// Step (a): runs `tlmgr args..` of `d` through `x` and captures the reply.
pub fn real_tlmgr(d: &Distribution, x: &dyn Exec, args: &[&str]) -> Captured {
    let mut spec = d.command("tlmgr");
    spec.args.extend(args.iter().map(Into::into));
    let out = x
        .run(&spec)
        .unwrap_or_else(|e| panic!("tlmgr {args:?}: {e}"));
    Captured::from_output(&out)
}

fn pkgs(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

/// What a caller sees from `info`: the rows, or the error's code.
fn info_outcome(
    x: &dyn Exec,
    d: &Distribution,
    names: &[&str],
) -> Result<Vec<Vec<String>>, String> {
    let cfg = Config::default();
    Tlmgr::new(d, &cfg, x, &NullObserver)
        .info(&pkgs(names), &["name"], true)
        .map_err(|e| format!("{:?}", e.code()))
}

/// Asserts the fake's reply conforms to the real one: same status class and,
/// when `rows` is set, the same stdout rows.
fn assert_conforms(case: &str, fake: &Captured, real: &Captured, rows: bool) {
    assert_eq!(
        fake.ok(),
        real.ok(),
        "{case}: status class differs\n  fake: {:?} {:?}\n  real: {:?} {:?}",
        fake.status,
        fake.stderr,
        real.status,
        real.stderr
    );
    if rows {
        assert_eq!(fake.rows(), real.rows(), "{case}: stdout rows differ");
    }
}

const INFO: [&str; 5] = ["info", "--list", "--only-installed", "--data", "name"];

fn info_args<'a>(names: &[&'a str]) -> Vec<&'a str> {
    let mut v: Vec<&str> = INFO.to_vec();
    v.extend(names);
    v
}

// ---------- step (a) ----------

#[test]
fn real_tlmgr_returns_the_status_and_both_streams() {
    let m = model(&["framed"], &[], &[]);
    m.raw("info", super::tlmgr_model::fixture("info-missing"));
    let x = fake(&m);
    let got = real_tlmgr(&dist("conf-a"), &x, &info_args(&["animate"]));
    assert_eq!(got, Captured::fixture("info-missing"));
}

#[test]
fn a_captured_run_saves_as_a_fixture_that_loads_back_identically() {
    let tmp = tempfile::tempdir().unwrap();
    let c = Captured::fixture("install-readonly");
    c.save(tmp.path(), "again");
    let read =
        |ext: &str| std::fs::read_to_string(tmp.path().join(format!("again.{ext}"))).unwrap();
    assert_eq!(read("status").trim(), "2");
    assert_eq!(read("stdout"), c.stdout);
    assert_eq!(read("stderr"), c.stderr);
}

// ---------- step (b): the fake against the captured fixtures ----------

mod offline {
    use super::*;

    /// One `info` case: the fake's reply and the crate's outcome against the
    /// fixture. `installed` is the model's state, `asked` the requested names.
    fn info_case(fixture: &str, installed: &[&str], asked: &[&str]) {
        let d = dist(&format!("conf-info-{fixture}"));
        let real = Captured::fixture(fixture);

        let m = model(installed, &[], &[]);
        let fake_reply = real_tlmgr(&d, &fake(&m), &info_args(asked));
        assert_conforms(fixture, &fake_reply, &real, true);

        // The crate's classification of the fake's reply equals its
        // classification of the real one (the real reply replayed through the
        // same model).
        let from_model = info_outcome(&fake(&model(installed, &[], &[])), &d, asked);
        let raw = model(installed, &[], &[]);
        raw.raw("info", super::super::tlmgr_model::fixture(fixture));
        let from_fixture = info_outcome(&fake(&raw), &d, asked);
        assert_eq!(
            from_model, from_fixture,
            "{fixture}: the crate's outcome differs"
        );
    }

    #[test]
    fn info_of_an_installed_package() {
        info_case("info-installed", &["framed"], &["framed"]);
    }

    #[test]
    fn info_of_a_missing_package() {
        info_case("info-missing", &[], &["animate"]);
    }

    #[test]
    fn info_of_a_mixed_set() {
        info_case("info-mixed", &["framed"], &["framed", "animate", "ocgx2"]);
    }

    #[test]
    fn info_of_an_unknown_package() {
        info_case("info-unknown", &[], &["nosuch_zzz"]);
    }

    #[test]
    fn install_of_a_package_that_is_present_exits_zero() {
        let m = model(&["framed"], &[], &[]);
        let got = real_tlmgr(&dist("conf-present"), &fake(&m), &["install", "framed"]);
        assert_conforms(
            "install-present",
            &got,
            &Captured::fixture("install-present"),
            false,
        );
    }

    #[test]
    fn a_successful_install_exits_zero_and_the_crate_reports_it_installed() {
        let d = dist("conf-install-ok");
        let both = ["animate", "ocgx2"];
        let m = model(&[], &both, &[]);
        let got = real_tlmgr(&d, &fake(&m), &["install", "animate", "ocgx2"]);
        assert_conforms("install-ok", &got, &Captured::fixture("install-ok"), false);

        let outcome = |raw: bool| {
            let m = model(&[], &both, &[]);
            if raw {
                m.raw("install", super::super::tlmgr_model::fixture("install-ok"));
            }
            let x = fake(&m);
            let cfg = Config::default();
            Tlmgr::new(&d, &cfg, &x, &NullObserver)
                .install(&pkgs(&both))
                .map_err(|e| format!("{:?}", e.code()))
        };
        assert_eq!(outcome(false), outcome(true));
        assert_eq!(outcome(true).unwrap().installed, both);
    }

    #[test]
    fn a_silent_rejection_exits_zero_and_the_crate_reports_the_package_failed() {
        // `pathrepo-bad`: a corrupt download exits 0 and installs nothing.
        let d = dist("conf-silent");
        let real = Captured::fixture("pathrepo-bad");
        assert!(real.ok(), "the real silent rejection exits 0");
        // The crate retries once (`update --self`, then a second install), so
        // a persistent rejection needs both installs scripted to do nothing.
        let silent = || model(&[], &["fakebad"], &[(0, false), (0, false)]);
        let got = real_tlmgr(&d, &fake(&silent()), &["install", "fakebad"]);
        assert_conforms("pathrepo-bad", &got, &real, false);

        let m = silent();
        m.raw(
            "install",
            super::super::tlmgr_model::fixture("pathrepo-bad"),
        );
        let x = fake(&m);
        let cfg = Config::default();
        let out = Tlmgr::new(&d, &cfg, &x, &NullObserver)
            .install(&pkgs(&["fakebad"]))
            .unwrap();
        assert_eq!(out.failed, ["fakebad"]);
        assert!(out.installed.is_empty());
    }

    #[test]
    fn a_search_that_finds_nothing_exits_zero_with_no_rows() {
        let d = dist("conf-search-empty");
        let real = Captured::fixture("search-empty");
        let got = real_tlmgr(
            &d,
            &fake(&model(&[], &[], &[])),
            &["search", "--file", "--global", "/nosuchfile-zzqq.sty"],
        );
        assert_conforms("search-empty", &got, &real, true);

        let found = |raw: bool| {
            let m = model(&[], &[], &[]);
            if raw {
                m.raw("search", super::super::tlmgr_model::fixture("search-empty"));
            }
            let x = fake(&m);
            let cfg = Config::default();
            Tlmgr::new(&d, &cfg, &x, &NullObserver)
                .search_file(&SearchTerm::File("/nosuchfile-zzqq.sty".to_owned()))
                .map_err(|e| format!("{:?}", e.code()))
        };
        assert_eq!(found(false), found(true));
        assert_eq!(found(true).unwrap(), Vec::<String>::new());
    }

    // ---- known divergences: the fake is wrong, the fixture is right ----

    #[test]
    #[ignore = "testing plan Phase 4: the model's `install` of an unknown package exits 0 and installs nothing; the real tool exits 1"]
    fn install_of_an_unknown_package_exits_nonzero() {
        let m = model(&[], &[], &[]);
        let got = real_tlmgr(&dist("conf-nosuch"), &fake(&m), &["install", "nosuch_zzz"]);
        assert_conforms(
            "install-nosuch",
            &got,
            &Captured::fixture("install-nosuch"),
            false,
        );
    }

    #[test]
    #[ignore = "testing plan Phase 4: the model has no `search`; the real tool fails with the year-mismatch text when the repository is newer"]
    fn a_search_against_a_newer_repository_exits_nonzero() {
        let m = model(&[], &[], &[]);
        let got = real_tlmgr(
            &dist("conf-older"),
            &fake(&m),
            &["search", "--file", "--global", "/ocgbase.sty"],
        );
        let real = Captured::fixture("search-older-remote");
        assert!(!real.ok());
        assert_conforms("search-older-remote", &got, &real, false);
    }
}

// ---------- step (c): the fake against the real tool ----------

mod real {
    use super::super::sandbox;
    use super::*;

    /// A TeX distribution for a read-only query, or `None` after printing why
    /// the test skips (a failure under `TINYTEX_RS_REQUIRE_TEX=1`).
    fn found(test: &str) -> Option<Distribution> {
        let d =
            tinytex::detect::find(&Config::default()).filter(|d| d.bin_dir.join("tlmgr").exists());
        if d.is_none() {
            assert!(
                std::env::var("TINYTEX_RS_REQUIRE_TEX").as_deref() != Ok("1"),
                "{test}: TINYTEX_RS_REQUIRE_TEX=1 but no TeX distribution was found"
            );
            eprintln!("SKIPPED {test}: no TeX distribution");
        }
        d
    }

    fn recapture(c: &Captured, name: &str) {
        if let Some(dir) = std::env::var_os("TINYTEX_RS_RECAPTURE_DIR") {
            c.save(Path::new(&dir), name);
        }
    }

    /// An installed package name, chosen at run time, and one that no
    /// distribution has.
    fn names(d: &Distribution, x: &dyn Exec) -> (String, &'static str) {
        let all = real_tlmgr(d, x, &INFO);
        assert!(
            all.ok(),
            "listing the installed packages failed: {}",
            all.stderr
        );
        let rows = all.rows();
        let pick = ["framed", "xcolor", "graphics"]
            .into_iter()
            .find(|p| rows.contains(*p))
            .map(str::to_owned)
            .or_else(|| rows.into_iter().next())
            .expect("the distribution lists at least one installed package");
        (pick, "tinytex-rs-absent-zzqq")
    }

    /// Read-only: the fake's `info` against the real one, for one installed
    /// name, one absent name, and both.
    #[test]
    fn info_matches_the_real_tlmgr() {
        let Some(d) = found("info_matches_the_real_tlmgr") else {
            return;
        };
        let x = StdExec::new();
        let (have, absent) = names(&d, &x);
        let fake_d = dist("conf-real-info");
        for (tag, asked) in [
            ("installed", vec![have.as_str()]),
            ("absent", vec![absent]),
            ("mixed", vec![have.as_str(), absent]),
        ] {
            let real = real_tlmgr(&d, &x, &info_args(&asked));
            recapture(&real, &format!("info-{tag}"));
            let m = model(&[have.as_str()], &[], &[]);
            let fake_reply = real_tlmgr(&fake_d, &fake(&m), &info_args(&asked));
            assert_conforms(tag, &fake_reply, &real, true);

            let real_out = info_outcome(&x, &d, &asked);
            let fake_out = info_outcome(&fake(&model(&[have.as_str()], &[], &[])), &fake_d, &asked);
            assert_eq!(real_out, fake_out, "{tag}: the crate's outcome differs");
        }
    }

    /// `is_installed` through the real tool and through the fake agree.
    #[test]
    fn is_installed_matches_the_real_tlmgr() {
        let Some(d) = found("is_installed_matches_the_real_tlmgr") else {
            return;
        };
        let x = StdExec::new();
        let (have, absent) = names(&d, &x);
        let cfg = Config::default();
        let real = Tlmgr::new(&d, &cfg, &x, &NullObserver);
        let fake_d = dist("conf-real-is-installed");
        let fx = fake(&model(&[have.as_str()], &[], &[]));
        let simulated = Tlmgr::new(&fake_d, &cfg, &fx, &NullObserver);
        assert!(real.is_installed(&have) && simulated.is_installed(&have));
        assert!(!real.is_installed(absent) && !simulated.is_installed(absent));
    }

    /// The `--repository` for calls that contact one: what Phase 1's script
    /// prints (nothing for the default), or `TINYTEX_RS_TEST_REPOSITORY`.
    fn repository_args(sb: &sandbox::Sandbox) -> Vec<String> {
        let url = match std::env::var("TINYTEX_RS_TEST_REPOSITORY") {
            Ok(u) => u,
            Err(_) => {
                let script = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join(".github/scripts/tinytex-repository.sh");
                let mut spec = tinytex::CommandSpec::new("/bin/sh");
                spec.args = vec![script.into_os_string()];
                spec.env.push(("PATH".into(), sb.exec_path()));
                let out = sb.exec().run(&spec).expect("run tinytex-repository.sh");
                assert!(
                    out.status.success(),
                    "tinytex-repository.sh failed: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
                String::from_utf8_lossy(&out.stdout).trim().to_owned()
            }
        };
        if url.is_empty() {
            Vec::new()
        } else {
            vec!["--repository".to_owned(), url]
        }
    }

    fn with_repo<'a>(repo: &'a [String], args: &[&'a str]) -> Vec<&'a str> {
        repo.iter()
            .map(String::as_str)
            .chain(args.iter().copied())
            .collect()
    }

    /// Inside a sandbox: `install` of a package that is already present.
    #[test]
    fn install_of_a_present_package_matches_the_real_tlmgr() {
        let Some(sb) = sandbox::gate("install_of_a_present_package_matches_the_real_tlmgr") else {
            return;
        };
        let (have, _) = names(&sb.dist, sb.exec());
        let repo = repository_args(&sb);
        let real = real_tlmgr(&sb.dist, sb.exec(), &with_repo(&repo, &["install", &have]));
        recapture(&real, "install-present");
        let m = model(&[have.as_str()], &[], &[]);
        let got = real_tlmgr(&dist("conf-real-present"), &fake(&m), &["install", &have]);
        assert_conforms("install-present", &got, &real, false);
    }

    /// Inside a sandbox: a search that finds nothing.
    #[test]
    fn a_search_that_finds_nothing_matches_the_real_tlmgr() {
        let Some(sb) = sandbox::gate("a_search_that_finds_nothing_matches_the_real_tlmgr") else {
            return;
        };
        let repo = repository_args(&sb);
        let args = ["search", "--file", "--global", "/nosuchfile-zzqq.sty"];
        let real = real_tlmgr(&sb.dist, sb.exec(), &with_repo(&repo, &args));
        recapture(&real, "search-empty");
        let got = real_tlmgr(
            &dist("conf-real-search"),
            &fake(&model(&[], &[], &[])),
            &args,
        );
        assert_conforms("search-empty", &got, &real, true);
    }

    /// Inside a sandbox: `install` of a package no repository has.
    #[test]
    #[ignore = "testing plan Phase 4: the model's `install` of an unknown package exits 0; the real tool exits 1"]
    fn install_of_an_unknown_package_matches_the_real_tlmgr() {
        let Some(sb) = sandbox::gate("install_of_an_unknown_package_matches_the_real_tlmgr") else {
            return;
        };
        let (_, absent) = names(&sb.dist, sb.exec());
        let repo = repository_args(&sb);
        let real = real_tlmgr(&sb.dist, sb.exec(), &with_repo(&repo, &["install", absent]));
        recapture(&real, "install-nosuch");
        let got = real_tlmgr(
            &dist("conf-real-nosuch"),
            &fake(&model(&[], &[], &[])),
            &["install", absent],
        );
        assert_conforms("install-nosuch", &got, &real, false);
    }
}
