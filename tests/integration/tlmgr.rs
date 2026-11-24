//! `Tlmgr` through a mocked `Exec`: recorded `CommandSpec` sequences for the
//! install policy of design section 3.2, the command forms, and the result
//! types. Cancellation is in `tlmgr_cancel`, the lock in `tlmgr_lock`.

use std::sync::Mutex;

use tinytex::log::SearchTerm;
use tinytex::test_support::{FakeTex, Reply};
use tinytex::tlmgr::share_lock;
use tinytex::{
    Config, Event, NullObserver, Observer, PathAction, Tlmgr, TlmgrError, TlmgrUpdate, UpdateOpts,
    UpdateOutcome,
};

use super::tlmgr_model::{cfg_with, dist, fake, model, opts_with, without_info};

fn pkgs(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

const INFO_AB: &str = "tlmgr info --list --only-installed --data name a b";

// ---------- install policy ----------

#[test]
fn install_is_one_deduplicated_batch_call() {
    let d = dist("batch");
    let m = model(&[], &["a", "b"], &[]);
    let x = fake(&m);
    let cfg = Config::default();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
    let out = t.install(&pkgs(&["a", "b", "a"])).unwrap();
    assert_eq!(without_info(x.lines()), ["tlmgr install a b"]);
    assert_eq!(out.installed, ["a", "b"]);
    assert!(out.already_present.is_empty() && out.failed.is_empty());
    // One check under the lock, one after the install.
    assert_eq!(x.lines().iter().filter(|l| *l == INFO_AB).count(), 2);
}

#[test]
fn install_status_255_is_accepted() {
    let d = dist("s255");
    let m = model(&[], &["a"], &[(255, true)]);
    let x = fake(&m);
    let cfg = Config::default();
    let out = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap();
    assert_eq!(without_info(x.lines()), ["tlmgr install a"]);
    assert_eq!(out.installed, ["a"]);
}

#[test]
fn another_status_runs_update_self_and_installs_again_once() {
    let d = dist("retry");
    let m = model(&[], &["a"], &[(1, false), (0, true)]);
    let x = fake(&m);
    let cfg = Config::default();
    let out = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap();
    assert_eq!(
        without_info(x.lines()),
        [
            "tlmgr install a",
            "tlmgr update --self",
            "fmtutil --sys --all",
            "tlmgr install a"
        ]
    );
    assert_eq!(out.installed, ["a"]);
    assert!(out.failed.is_empty());
}

#[test]
fn a_package_missing_after_status_zero_retries_and_a_second_miss_is_failed() {
    let d = dist("miss");
    let m = model(&[], &[], &[]);
    let x = fake(&m);
    let cfg = Config::default();
    let out = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap();
    assert_eq!(x.count("tlmgr install"), 2, "one retry, never a third");
    assert_eq!(x.count("tlmgr update --self"), 1);
    assert_eq!(out.failed, ["a"]);
    assert!(out.installed.is_empty());
}

#[test]
fn the_pre_install_update_needs_auto_and_an_already_installed_package() {
    let cfg = Config::default();

    // Auto, `a` present, `b` not: update first, then the batch install.
    let d = dist("pre-auto");
    let m = model(&["a"], &["b"], &[]);
    let x = fake(&m);
    let out = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a", "b"]))
        .unwrap();
    assert_eq!(
        without_info(x.lines()),
        [
            "tlmgr update --all --self",
            "fmtutil --sys --all",
            "tlmgr install a b"
        ]
    );
    assert_eq!(out.already_present, ["a"]);
    assert_eq!(out.installed, ["b"]);

    // Auto, nothing present: no update.
    let d = dist("pre-none");
    let x = fake(&model(&[], &["a", "b"], &[]));
    Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a", "b"]))
        .unwrap();
    assert_eq!(without_info(x.lines()), ["tlmgr install a b"]);

    // Never, `a` present: still no update.
    let never = cfg_with(|c| c.tlmgr_update = TlmgrUpdate::Never);
    let d = dist("pre-never");
    let x = fake(&model(&["a"], &["b"], &[]));
    Tlmgr::new(&d, &never, &x, &NullObserver)
        .install(&pkgs(&["a", "b"]))
        .unwrap();
    assert_eq!(without_info(x.lines()), ["tlmgr install a b"]);
}

#[test]
fn install_returns_early_when_every_package_is_present() {
    let d = dist("allpresent");
    let x = fake(&model(&["a", "b"], &[], &[]));
    let cfg = Config::default();
    let out = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a", "b"]))
        .unwrap();
    assert!(without_info(x.lines()).is_empty(), "{:?}", x.lines());
    assert_eq!(out.already_present, ["a", "b"]);
}

#[test]
fn never_issues_no_update_even_after_a_failed_install() {
    let cfg = cfg_with(|c| c.tlmgr_update = TlmgrUpdate::Never);
    let d = dist("never-fail");
    let x = fake(&model(&[], &[], &[(1, false)]));
    let out = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap();
    assert_eq!(without_info(x.lines()), ["tlmgr install a"]);
    assert_eq!(out.failed, ["a"]);
    // An explicit update is skipped too.
    let r = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .update(UpdateOpts::default())
        .unwrap();
    assert_eq!(r, UpdateOutcome::Skipped);
    assert_eq!(x.count("tlmgr update"), 0);
}

#[test]
fn at_most_one_update_per_distribution_per_process_but_the_install_retry_is_exempt() {
    let cfg = Config::default();
    let d = dist("once");

    // First: pre-install update runs.
    let x1 = fake(&model(&["a"], &["b"], &[]));
    Tlmgr::new(&d, &cfg, &x1, &NullObserver)
        .install(&pkgs(&["a", "b"]))
        .unwrap();
    assert_eq!(x1.count("tlmgr update"), 1);

    // Second, a fresh Tlmgr on the same distribution: the pre-install update
    // is skipped, because one already ran in this process.
    let x2 = fake(&model(&["c"], &["d"], &[]));
    Tlmgr::new(&d, &cfg, &x2, &NullObserver)
        .install(&pkgs(&["c", "d"]))
        .unwrap();
    assert_eq!(x2.count("tlmgr update"), 0);

    // An explicit update is skipped as well.
    let r = Tlmgr::new(&d, &cfg, &x2, &NullObserver)
        .update(UpdateOpts::default())
        .unwrap();
    assert_eq!(r, UpdateOutcome::Skipped);

    // But the `update --self` retry inside install is not subject to the flag.
    let x3 = fake(&model(&[], &["e"], &[(1, false), (0, true)]));
    Tlmgr::new(&d, &cfg, &x3, &NullObserver)
        .install(&pkgs(&["e"]))
        .unwrap();
    assert_eq!(x3.count("tlmgr update --self"), 1);
}

#[test]
fn two_compiles_that_each_hit_a_missing_package_issue_one_update_in_total() {
    // What plan 5.4a does per compile: update on the first engine failure
    // (lazy, Auto), then install what the log names.
    let cfg = Config::default();
    let d = dist("two-compiles");
    let mut total = 0;
    for pkg in ["a", "b"] {
        let x = fake(&model(&[], &["a", "b"], &[]));
        let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
        t.update(opts_with(|o| o.run_fmtutil = false)).unwrap();
        t.install(&pkgs(&[pkg])).unwrap();
        total += x.count("tlmgr update");
    }
    assert_eq!(total, 1);
}

#[test]
fn texlive_too_old_is_reported_once_and_skips_every_later_update() {
    let cfg = Config::default();
    let d = dist("tooold");
    let x = FakeTex::new();
    x.reply(
        "tlmgr",
        &["update"],
        Reply::code(1)
            .stderr("tlmgr: Local TeX Live (2024) is older than remote repository (2025).\n"),
    );
    let r = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .update(UpdateOpts::default())
        .unwrap();
    assert_eq!(
        r,
        UpdateOutcome::TexLiveTooOld {
            local: 2024,
            remote: 2025
        }
    );
    assert_eq!(
        x.count("fmtutil"),
        0,
        "no format rebuild when TeX Live is too old"
    );

    // Later updates, from another Tlmgr, run nothing and say the same.
    let x2 = FakeTex::new();
    let r = Tlmgr::new(&d, &cfg, &x2, &NullObserver)
        .update(UpdateOpts::default())
        .unwrap();
    assert!(matches!(
        r,
        UpdateOutcome::TexLiveTooOld {
            local: 2024,
            remote: 2025
        }
    ));
    assert!(x2.calls().is_empty());

    // An install that fails and cannot update reports TexLiveTooOld, and does
    // not install a second time.
    let x3 = fake(&model(&[], &[], &[(1, false)]));
    let err = Tlmgr::new(&d, &cfg, &x3, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap_err();
    assert!(
        matches!(
            err,
            TlmgrError::TexLiveTooOld {
                local: 2024,
                remote: 2025
            }
        ),
        "{err:?}"
    );
    assert_eq!(x3.count("tlmgr install"), 1);
    assert_eq!(x3.count("tlmgr update"), 0);
}

#[test]
fn install_never_issues_tlmgr_path_add() {
    let cfg = Config::default();
    for (name, script) in [
        ("nopath-ok", vec![]),
        ("nopath-retry", vec![(1, false), (0, true)]),
    ] {
        let d = dist(name);
        let x = fake(&model(&["a"], &["a", "b"], &script));
        Tlmgr::new(&d, &cfg, &x, &NullObserver)
            .install(&pkgs(&["a", "b"]))
            .unwrap();
        assert_eq!(x.count("tlmgr path"), 0, "{:?}", x.lines());
        assert_eq!(x.count("osascript"), 0);
    }
}

// ---------- fmtutil ----------

// The failure message and its three cases are seeded from Quarto 1's
// `tests/unit/latexmk/fmtutil-handler.test.ts` at v1.12.3 (`fmtutilFailureMessage`
// in `src/command/render/latexmk/texlive.ts`), Copyright (C) 2026 Posit
// Software, PBC; the fixture text is copied with attribution.

#[test]
fn fmtutil_succeeded_is_ok() {
    let d = dist("fmt-ok");
    let x = FakeTex::new();
    let cfg = Config::default();
    assert!(Tlmgr::new(&d, &cfg, &x, &NullObserver).fmtutil().is_ok());
    assert_eq!(x.lines(), ["fmtutil --sys --all"]);
}

#[test]
fn fmtutil_failure_is_a_warning_with_the_command_and_stderr() {
    let d = dist("fmt-fail");
    let x = FakeTex::new();
    x.reply(
        "fmtutil",
        &[],
        Reply::code(1).stderr("fmtutil: error rebuilding format luatex"),
    );
    let cfg = Config::default();
    let w = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .fmtutil()
        .unwrap_err();
    assert_eq!(w.exit_code, Some(1));
    assert!(w.message.contains("fmtutil-sys --all"), "{}", w.message);
    assert!(w.message.contains("non-fatal"), "{}", w.message);
    assert!(
        w.message
            .contains("fmtutil: error rebuilding format luatex"),
        "{}",
        w.message
    );
}

#[test]
fn fmtutil_warning_omits_a_blank_stderr_suffix() {
    let d = dist("fmt-blank");
    let x = FakeTex::new();
    x.reply("fmtutil", &[], Reply::code(1).stderr("   \n"));
    let cfg = Config::default();
    let w = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .fmtutil()
        .unwrap_err();
    assert_eq!(
        w.message,
        "Failed to rebuild format tree (`fmtutil-sys --all` exited 1). This is non-fatal \u{2014} package installation will continue."
    );
}

#[test]
fn a_failed_fmtutil_during_update_and_install_never_becomes_an_error() {
    let cfg = Config::default();

    let d = dist("fmt-update");
    let x = FakeTex::new();
    x.reply("fmtutil", &[], Reply::code(2).stderr("bad format"));
    let r = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .update(UpdateOpts::default())
        .unwrap();
    let UpdateOutcome::Updated { warning: Some(w) } = r else {
        panic!("{r:?}")
    };
    assert_eq!(w.exit_code, Some(2));

    let d = dist("fmt-install");
    let m = model(&[], &["a"], &[(1, false), (0, true)]);
    let x = fake(&m);
    x.reply("fmtutil", &[], Reply::code(2).stderr("bad format"));
    let out = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap();
    assert_eq!(out.installed, ["a"]);
    assert_eq!(out.warnings.len(), 1);
}

// ---------- command forms ----------

#[test]
fn windows_forms_use_no_temporary_script() {
    let d = dist("windows");
    let cfg = Config::default();
    let x = FakeTex::new();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver).for_windows(true);
    t.update(UpdateOpts::default()).unwrap();
    for spec in x.calls() {
        let program = spec.program.to_string_lossy().into_owned();
        let all: Vec<String> = std::iter::once(program.clone())
            .chain(spec.args.iter().map(|a| a.to_string_lossy().into_owned()))
            .collect();
        for s in &all {
            assert!(
                !s.contains(".tmp") && !s.ends_with(".cmd") && !s.contains("Temp"),
                "a temporary script: {s}"
            );
        }
    }
    let calls = x.calls();
    assert!(
        calls[0].program.ends_with("tlmgr.bat"),
        "{:?}",
        calls[0].program
    );
    assert_eq!(calls[1].program, std::path::PathBuf::from("fmtutil"));
    assert_eq!(
        calls[1].args,
        ["--sys", "--all"],
        "the Windows form already carries its arguments; they are not doubled"
    );
}

#[test]
fn unix_fmtutil_is_fmtutil_sys_all_with_the_installation_flag() {
    let d = dist("unix-fmt");
    let cfg = Config::default();
    let x = FakeTex::new();
    Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .for_windows(false)
        .fmtutil()
        .unwrap();
    let call = &x.calls()[0];
    assert_eq!(call.program, d.bin_dir.join("fmtutil"));
    assert_eq!(call.args, ["--sys", "--all"]);
    assert!(call.changes_installation);
    assert!(
        call.env
            .iter()
            .any(|(k, v)| k == "TEXLIVE_PREFER_OWN" && v == "1")
    );
}

#[test]
fn usermode_and_tlmgr_opts_precede_the_action() {
    let cfg = cfg_with(|c| {
        c.usermode = true;
        c.tlmgr_opts = pkgs(&["--repository", "https://example.org/tlnet"]);
    });
    let d = dist("opts");
    let x = FakeTex::new();
    Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .update(UpdateOpts::default())
        .unwrap();
    assert_eq!(
        x.lines()[0],
        "tlmgr --usermode --repository https://example.org/tlnet update --all",
        "no --self in user mode"
    );
}

#[test]
fn update_arguments_follow_the_options() {
    let cfg = Config::default();
    let d = dist("upd-args");
    let x = FakeTex::new();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
    t.run(&["--version"], false).unwrap();
    // Build several updates on distinct distributions so the once-flag does
    // not interfere.
    for (name, opts, expect) in [
        ("u1", opts_with(|_| {}), "tlmgr update --all --self"),
        (
            "u2",
            opts_with(|o| o.self_only = true),
            "tlmgr update --self",
        ),
        (
            "u3",
            opts_with(|o| {
                o.all = false;
                o.more_args = pkgs(&["--no-auto-remove"]);
            }),
            "tlmgr update --no-auto-remove",
        ),
    ] {
        let d = dist(name);
        let x = FakeTex::new();
        Tlmgr::new(&d, &cfg, &x, &NullObserver)
            .update(opts)
            .unwrap();
        assert_eq!(x.lines()[0], expect);
    }
}

#[test]
fn delete_tlpdb_removes_only_the_hashed_database_copies() {
    let root = tempfile::tempdir().unwrap();
    let tlpkg = root.path().join("tlpkg");
    std::fs::create_dir_all(&tlpkg).unwrap();
    let hashed = tlpkg.join("texlive.tlpdb.0123456789abcdef0123456789abcdef");
    let keep = [
        tlpkg.join("texlive.tlpdb"),
        tlpkg.join("texlive.tlpdb.main"),
    ];
    std::fs::write(&hashed, "x").unwrap();
    for k in &keep {
        std::fs::write(k, "x").unwrap();
    }
    let mut d = dist("tlpdb");
    d.root = Some(root.path().to_path_buf());
    let cfg = Config::default();
    let x = FakeTex::new();
    Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .update(opts_with(|o| o.delete_tlpdb = true))
        .unwrap();
    assert!(!hashed.exists());
    assert!(keep.iter().all(|k| k.exists()));
}

// ---------- classification ----------

#[test]
fn commands_that_change_the_installation_are_marked_and_read_only_ones_are_not() {
    let cfg = Config::default();
    let d = dist("classify");
    let x = FakeTex::new();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);

    // Read-only.
    let _ = t.version();
    let _ = t.search_file(&SearchTerm::File("/framed.sty".into()));
    let _ = t.info(&pkgs(&["a"]), &["name"], true);
    let _ = t.repository();
    let _ = t.is_installed("a");
    assert!(!x.calls().is_empty());
    assert!(
        x.calls().iter().all(|c| !c.changes_installation),
        "{:?}",
        x.lines()
    );

    // Changing.
    type Change = Box<dyn Fn(&Tlmgr)>;
    let changing: Vec<(&str, Change)> = vec![
        (
            "update",
            Box::new(|t| drop(t.update(UpdateOpts::default()))),
        ),
        ("remove", Box::new(|t| drop(t.remove(&pkgs(&["a"]))))),
        (
            "path",
            Box::new(|t| drop(t.path_with(PathAction::Add, None))),
        ),
        (
            "option",
            Box::new(|t| drop(t.set_repository("https://example.org"))),
        ),
        ("fmtutil", Box::new(|t| drop(t.fmtutil()))),
    ];
    for (i, (what, call)) in changing.iter().enumerate() {
        let d = dist(&format!("classify-{i}"));
        let x = FakeTex::new();
        let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
        call(&t);
        let calls = x.calls();
        assert!(!calls.is_empty(), "{what}");
        assert!(
            calls.iter().all(|c| c.changes_installation),
            "{what}: {:?}",
            x.lines()
        );
    }

    // Install: the install command itself.
    let d = dist("classify-install");
    let x = fake(&model(&[], &["a"], &[]));
    Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap();
    let flags: Vec<(String, bool)> = x
        .calls()
        .iter()
        .zip(x.lines())
        .map(|(c, l)| (l, c.changes_installation))
        .collect();
    for (line, flag) in flags {
        assert_eq!(flag, line.starts_with("tlmgr install"), "{line}");
    }
}

#[test]
fn run_uses_the_flag_its_caller_passes() {
    let cfg = Config::default();
    let d = dist("run-flag");
    let x = FakeTex::new();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
    t.run(&["conf", "auxtrees", "add", "/x"], true).unwrap();
    t.run(&["--version"], false).unwrap();
    let calls = x.calls();
    assert!(calls[0].changes_installation);
    assert!(!calls[1].changes_installation);
    assert_eq!(x.lines(), ["tlmgr conf auxtrees add /x", "tlmgr --version"]);
}

// ---------- read-only commands ----------

#[test]
fn version_parses_the_year_and_date() {
    let d = dist("version");
    let x = FakeTex::new();
    x.reply(
        "tlmgr",
        &["--version"],
        Reply::ok().stdout(
            "tlmgr revision 74330 (2025-02-15 17:07:49 +0100)\ntlmgr using installation: /x\nTeX Live (https://tug.org/texlive) version 2025\n",
        ),
    );
    let cfg = Config::default();
    let v = Tlmgr::new(&d, &cfg, &x, &NullObserver).version().unwrap();
    assert_eq!(v.year, 2025);
    assert_eq!(v.date, "2025-02-15");

    let x = FakeTex::new();
    x.reply("tlmgr", &["--version"], Reply::ok().stdout("nonsense"));
    let err = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .version()
        .unwrap_err();
    assert!(matches!(err, TlmgrError::Unparseable { .. }), "{err:?}");
}

const SEARCH_MF: &str = "tlmgr: package repository https://mirror.ctan.org/systems/texlive/tlnet (verified)\nmetafont.x86_64-darwin:\n\tbin/x86_64-darwin/mf\nmetapost.x86_64-darwin:\n\tbin/x86_64-darwin/mfplain\nframed:\n\ttexmf-dist/tex/latex/framed/mf\n";

#[test]
fn search_file_returns_packages_with_a_file_ending_in_the_term() {
    let d = dist("search");
    let x = FakeTex::new();
    x.reply("tlmgr", &["search"], Reply::ok().stdout(SEARCH_MF));
    let cfg = Config::default();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
    let r = t.search_file(&SearchTerm::File("/mf".into())).unwrap();
    assert_eq!(r, ["metafont", "framed"], "mfplain does not end in /mf");
    assert_eq!(x.lines(), ["tlmgr search --file --global /mf"]);
}

#[test]
fn search_file_needs_no_command_for_a_named_package() {
    let d = dist("search-pkg");
    let x = FakeTex::new();
    let cfg = Config::default();
    let r = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .search_file(&SearchTerm::Package("fandol".into()))
        .unwrap();
    assert_eq!(r, ["fandol"]);
    assert!(x.calls().is_empty());
}

#[test]
fn info_returns_the_requested_fields_per_package() {
    let d = dist("info");
    let x = FakeTex::new();
    x.reply(
        "tlmgr",
        &["info"],
        Reply::ok().stdout("framed,12345\nxcolor,678\n"),
    );
    let cfg = Config::default();
    let rows = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .info(&pkgs(&["framed", "xcolor"]), &["name", "size"], true)
        .unwrap();
    assert_eq!(rows, [["framed", "12345"], ["xcolor", "678"]]);
    assert_eq!(
        x.lines(),
        ["tlmgr info --list --only-installed --data name,size framed xcolor"]
    );
}

/// What real `tlmgr info --only-installed` prints when the one package asked
/// about is not installed: status 1, nothing on stdout, a complaint on stderr.
const NOT_LOCALLY: &str = "tlmgr: package animate not locally!\n\
     tlmgr: action info returned an error; continuing.\n\
     tlmgr: An error has occurred. See above messages. Exiting.\n";

#[test]
fn info_of_one_unknown_package_is_an_empty_list() {
    let d = dist("info-unknown");
    let x = FakeTex::new();
    x.reply("tlmgr", &["info"], Reply::code(1).stderr(NOT_LOCALLY));
    let cfg = Config::default();
    let rows = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .info(&pkgs(&["animate"]), &["name"], true)
        .unwrap();
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn info_of_unknown_packages_without_only_installed_is_also_an_empty_list() {
    let d = dist("info-unknown-remote");
    let x = FakeTex::new();
    x.reply(
        "tlmgr",
        &["info"],
        Reply::code(1).stderr(
            "tlmgr: package nosuchpkg not found neither locally nor remote!\n\
             tlmgr: action info returned an error; continuing.\n",
        ),
    );
    let cfg = Config::default();
    let rows = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .info(&pkgs(&["nosuchpkg"]), &["name"], false)
        .unwrap();
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn info_still_fails_when_tlmgr_fails_for_another_reason() {
    let d = dist("info-broken");
    let x = FakeTex::new();
    x.reply(
        "tlmgr",
        &["info"],
        Reply::code(1).stderr("tlmgr: cannot setup TLPDB in /nonexistent\n"),
    );
    let cfg = Config::default();
    let err = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .info(&pkgs(&["animate"]), &["name"], true)
        .unwrap_err();
    assert!(matches!(err, TlmgrError::Command { .. }), "{err:?}");
}

#[test]
fn is_installed_is_false_for_a_package_tlmgr_does_not_know() {
    let d = dist("is-installed");
    let x = FakeTex::new();
    x.reply("tlmgr", &["info"], Reply::code(1).stderr(NOT_LOCALLY));
    let cfg = Config::default();
    assert!(!Tlmgr::new(&d, &cfg, &x, &NullObserver).is_installed("animate"));
}

#[test]
fn repository_returns_the_url() {
    let d = dist("repo");
    let x = FakeTex::new();
    x.reply(
        "tlmgr",
        &["option", "repository"],
        Reply::ok().stdout("Default package repository (repository): https://mirror.ctan.org/systems/texlive/tlnet\n"),
    );
    let cfg = Config::default();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
    assert_eq!(
        t.repository().unwrap(),
        "https://mirror.ctan.org/systems/texlive/tlnet"
    );
    t.set_repository("https://example.org/tlnet").unwrap();
    assert_eq!(
        x.lines()[1],
        "tlmgr option repository https://example.org/tlnet"
    );
}

#[test]
fn is_installed_reads_the_name_list() {
    let d = dist("is-installed");
    let x = fake(&model(&["framed"], &[], &[]));
    let cfg = Config::default();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
    assert!(t.is_installed("framed"));
    assert!(!t.is_installed("xcolor"));
}

#[test]
fn packages_and_installs_for_a_log() {
    let log = "! LaTeX Error: File `framed.sty' not found.\n\nType X to quit.\n";
    let d = dist("for-log");
    let m = model(&[], &["framed"], &[]);
    let x = fake(&m);
    x.reply(
        "tlmgr",
        &["search"],
        Reply::ok().stdout("framed:\n\ttexmf-dist/tex/latex/framed/framed.sty\n"),
    );
    let cfg = Config::default();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
    assert_eq!(t.packages_for_log(log, None).unwrap(), ["framed"]);
    assert_eq!(x.lines(), ["tlmgr search --file --global /framed.sty"]);
    let out = t.install_for_log(log, None).unwrap();
    assert_eq!(out.installed, ["framed"]);
    assert_eq!(x.count("tlmgr install framed"), 1);

    let none = t.install_for_log("all fine\n", None).unwrap();
    assert!(none.installed.is_empty() && none.failed.is_empty());
}

// ---------- path ----------

#[test]
fn path_runs_tlmgr_path_or_writes_the_macos_paths_d_file() {
    let cfg = Config::default();
    let mut d = dist("path");
    // A native Windows path: the AppleScript literal doubles its backslashes,
    // so the check below must compare against the escaped form. Setting it
    // here makes the test fail the same way on every platform.
    d.bin_dir = std::path::PathBuf::from(r"C:\TinyTeX\bin\windows");
    let x = FakeTex::new();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
    t.path_with(PathAction::Add, None).unwrap();
    t.path_with(PathAction::Remove, None).unwrap();
    assert_eq!(x.lines(), ["tlmgr path add", "tlmgr path remove"]);

    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("TinyTeX");
    let x = FakeTex::new();
    let t = Tlmgr::new(&d, &cfg, &x, &NullObserver);
    t.path_with(PathAction::Add, Some(&file)).unwrap();
    let call = &x.calls()[0];
    assert_eq!(call.program, std::path::PathBuf::from("osascript"));
    assert!(call.changes_installation);
    let script = call
        .args
        .iter()
        .map(|a| a.to_string_lossy())
        .collect::<String>();
    // The script is an AppleScript literal, which doubles backslashes.
    let escaped = d.bin_dir.to_string_lossy().replace('\\', r"\\");
    assert!(script.contains(&escaped), "{script}");
    assert!(script.contains("administrator privileges"), "{script}");

    // The file already holds the directory: nothing runs.
    std::fs::write(&file, format!("{}\n", d.bin_dir.display())).unwrap();
    let x = FakeTex::new();
    Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .path_with(PathAction::Add, Some(&file))
        .unwrap();
    assert!(x.calls().is_empty());

    // Remove deletes it.
    Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .path_with(PathAction::Remove, Some(&file))
        .unwrap();
    let script = x.calls()[0]
        .args
        .iter()
        .map(|a| a.to_string_lossy())
        .collect::<String>();
    assert!(script.contains("rm -f"), "{script}");
}

// ---------- registry keys ----------

#[test]
fn the_registry_key_is_the_root_when_known_and_the_bin_dir_otherwise() {
    let mut a = dist("key-a");
    let mut b = dist("key-a");
    b.bin_dir = std::path::PathBuf::from("/nonexistent/elsewhere/bin");
    assert!(share_lock(&a, &b), "same root: one entry");

    let c = dist("key-c");
    assert!(!share_lock(&a, &c), "different roots: different entries");

    a.root = None;
    let mut e = a.clone();
    e.tlmgr = std::path::PathBuf::from("/x/tlmgr");
    assert!(share_lock(&a, &e), "no root, same bin_dir: one entry");
    let mut f = a.clone();
    f.bin_dir = std::path::PathBuf::from("/nonexistent/other-bin");
    assert!(!share_lock(&a, &f), "no root, different bin_dir");
    assert!(!share_lock(&a, &c));
}

// ---------- events ----------

#[derive(Default)]
struct Recorder(Mutex<Vec<Event>>);
impl Observer for Recorder {
    fn event(&self, e: &Event) {
        self.0.lock().unwrap().push(e.clone());
    }
}

#[test]
fn tlmgr_level_events_are_emitted() {
    let cfg = Config::default();
    let d = dist("events");
    let x = fake(&model(&["a"], &["b", "c"], &[]));
    let rec = Recorder::default();
    let t = Tlmgr::new(&d, &cfg, &x, &rec);
    t.install(&pkgs(&["a", "b", "c"])).unwrap();
    let events = rec.0.lock().unwrap().clone();
    assert_eq!(
        events,
        [
            Event::UpdatingTlmgr,
            Event::UpdatingPackages,
            Event::RebuildingFormats,
            Event::InstallingPackage {
                name: "b".into(),
                index: 1,
                of: 2
            },
            Event::InstallingPackage {
                name: "c".into(),
                index: 2,
                of: 2
            },
            Event::PackagesInstalled {
                names: pkgs(&["b", "c"])
            },
        ]
    );
    let rec = Recorder::default();
    Tlmgr::new(&d, &cfg, &FakeTex::new(), &rec)
        .search_file(&SearchTerm::File("/framed.sty".into()))
        .unwrap();
    assert_eq!(
        rec.0.lock().unwrap().clone(),
        [Event::Searching {
            term: SearchTerm::File("/framed.sty".into())
        }]
    );
}
