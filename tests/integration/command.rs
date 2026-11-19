//! `Distribution::command()`: recorded `CommandSpec`s. The Windows forms are
//! pure functions over a flag, so they run on every platform.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use tinytex::detect::filter_rtools;
use tinytex::{Distribution, Kind};

fn dist() -> Distribution {
    Distribution {
        root: Some(PathBuf::from("/opt/TinyTeX")),
        bin_dir: PathBuf::from("/opt/TinyTeX/bin/x86_64-linux"),
        tlmgr: PathBuf::from("/opt/TinyTeX/bin/x86_64-linux/tlmgr"),
        kind: Kind::TinyTeX,
    }
}

fn env_of<'a>(spec: &'a tinytex::CommandSpec, name: &str) -> Option<&'a OsString> {
    spec.env
        .iter()
        .find(|(k, _)| k == OsStr::new(name))
        .map(|(_, v)| v)
}

#[test]
fn path_is_prepended_with_bin_dir_for_every_program() {
    for program in [
        "tlmgr",
        "fmtutil",
        "updmap",
        "pdflatex",
        "bibtex",
        "makeindex",
    ] {
        let spec = dist().command_for(program, false, Some(OsStr::new("/usr/bin:/bin")));
        let path = env_of(&spec, "PATH").unwrap_or_else(|| panic!("{program}: no PATH"));
        assert_eq!(
            path.to_str().unwrap(),
            "/opt/TinyTeX/bin/x86_64-linux:/usr/bin:/bin",
            "{program}"
        );
    }
}

#[test]
fn prefer_own_is_set_for_tlmgr_fmtutil_and_updmap_only() {
    for program in ["tlmgr", "fmtutil", "updmap"] {
        let spec = dist().command_for(program, false, None);
        assert_eq!(
            env_of(&spec, "TEXLIVE_PREFER_OWN").map(|v| v.to_str().unwrap()),
            Some("1"),
            "{program}"
        );
    }
    for program in [
        "pdflatex",
        "xelatex",
        "lualatex",
        "bibtex",
        "biber",
        "makeindex",
        "latexmk",
    ] {
        let spec = dist().command_for(program, false, None);
        assert!(env_of(&spec, "TEXLIVE_PREFER_OWN").is_none(), "{program}");
    }
}

#[test]
fn env_holds_overrides_only_and_the_inherited_environment_survives() {
    // The spec carries only PATH (and TEXLIVE_PREFER_OWN): never a copy of
    // the whole environment.
    let spec = dist().command_for("pdflatex", false, Some(OsStr::new("/usr/bin")));
    let names: Vec<_> = spec
        .env
        .iter()
        .map(|(k, _)| k.to_str().unwrap().to_owned())
        .collect();
    assert_eq!(names, ["PATH"]);

    // Applied with `Command::env` on top of the inherited environment, a
    // variable the spec does not mention survives in the child.
    let probe = if cfg!(windows) {
        "echo %TINYTEX_TEST_INHERITED%"
    } else {
        "echo $TINYTEX_TEST_INHERITED"
    };
    let (shell, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let mut cmd = std::process::Command::new(shell);
    cmd.args([flag, probe])
        .env("TINYTEX_TEST_INHERITED", "kept");
    for (k, v) in &spec.env {
        if k != "PATH" {
            cmd.env(k, v);
        }
    }
    let out = cmd.output().unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("kept"));
}

#[test]
fn windows_forms_fmtutil_updmap_and_tlmgr_bat() {
    let d = dist();
    let fmtutil = d.command_for("fmtutil", true, None);
    assert_eq!(fmtutil.program, PathBuf::from("fmtutil"));
    assert_eq!(fmtutil.args, ["--sys", "--all"].map(OsString::from));
    assert!(fmtutil.changes_installation);

    let updmap = d.command_for("updmap", true, None);
    assert_eq!(updmap.program, PathBuf::from("updmap"));
    assert_eq!(updmap.args, ["--sys"].map(OsString::from));

    let tlmgr = d.command_for("tlmgr", true, None);
    assert_eq!(tlmgr.program, d.bin_dir.join("tlmgr.bat"));
    assert!(tlmgr.args.is_empty());
}

#[test]
fn unix_forms_are_plain_programs_under_bin_dir() {
    let d = dist();
    for program in ["tlmgr", "fmtutil", "updmap"] {
        let spec = d.command_for(program, false, None);
        assert_eq!(spec.program, d.bin_dir.join(program));
        assert!(spec.args.is_empty(), "{program}");
    }
}

#[test]
fn no_spec_names_a_temporary_script() {
    for program in ["tlmgr", "fmtutil", "updmap", "pdflatex"] {
        for windows in [false, true] {
            let spec = dist().command_for(program, windows, None);
            let all = format!("{:?} {:?}", spec.program, spec.args).to_lowercase();
            for banned in ["cmd", "tmp", "temp", ".ps1"] {
                assert!(!all.contains(banned), "{program} windows={windows}: {all}");
            }
        }
    }
}

#[test]
fn rtools_filter_is_a_pure_function_over_a_windows_path() {
    let path = r"C:\rtools44\usr\bin;C:\Windows\system32;C:\Rtools\mingw64\bin;D:\tools\RTOOLS45;C:\Program Files\R\bin;C:\myrtools\bin";
    assert_eq!(
        filter_rtools(path),
        r"C:\Windows\system32;C:\Program Files\R\bin;C:\myrtools\bin"
    );
    assert_eq!(filter_rtools(r"C:\a;C:\b"), r"C:\a;C:\b");
    assert_eq!(filter_rtools("C:/rtools43/x;C:/ok"), "C:/ok");
}

#[test]
fn on_windows_rtools_leaves_the_path_of_tlmgr_fmtutil_and_updmap_only() {
    let inherited = OsString::from(r"C:\rtools44\usr\bin;C:\Windows");
    for program in ["tlmgr", "fmtutil", "updmap"] {
        let spec = dist().command_for(program, true, Some(&inherited));
        let path = env_of(&spec, "PATH").unwrap().to_str().unwrap().to_owned();
        assert!(!path.to_lowercase().contains("rtools"), "{program}: {path}");
        assert!(path.ends_with(r"C:\Windows"), "{path}");
        assert!(path.starts_with("/opt/TinyTeX/bin/x86_64-linux;"), "{path}");
    }
    let engine = dist().command_for("pdflatex", true, Some(&inherited));
    assert!(
        env_of(&engine, "PATH")
            .unwrap()
            .to_str()
            .unwrap()
            .contains("rtools44")
    );
}
