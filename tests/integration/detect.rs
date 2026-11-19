//! Detection over temporary directory layouts, `tex_live_year`, `is_writable`.

use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::Mutex;

use tempfile::TempDir;
use tinytex::detect::{HostEnv, HostOs, default_install_dir_for, find_with};
use tinytex::test_support::exit_status;
use tinytex::{CommandSpec, Config, Distribution, Exec, Kind, Prefer};

const WINDOWS: bool = cfg!(windows);

fn tlmgr_name() -> &'static str {
    if WINDOWS { "tlmgr.bat" } else { "tlmgr" }
}

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}
#[cfg(not(unix))]
fn make_executable(_p: &Path) {}

/// `<root>/bin/<arch>/tlmgr`, returning the bin directory.
fn fake_install(root: &Path) -> PathBuf {
    let bin = root.join("bin").join("x86_64-linux");
    std::fs::create_dir_all(&bin).unwrap();
    let t = bin.join(tlmgr_name());
    std::fs::write(&t, "#!/bin/sh\n").unwrap();
    make_executable(&t);
    bin
}

fn cfg_dir(dir: &Path) -> Config {
    let mut c = Config::default();
    c.install_dir = Some(dir.to_path_buf());
    c
}

// ---- is_tinytex ----

#[test]
fn is_tinytex_by_directory_name() {
    let tmp = TempDir::new().unwrap();
    for (name, expect) in [
        ("TinyTeX", true),
        (".TinyTeX", true),
        ("tinytex", true),
        ("texlive", false),
        ("MyTinyTeX2", false),
    ] {
        let root = tmp.path().join(name);
        let bin = fake_install(&root);
        let d = Distribution::from_bin_dir(bin);
        assert_eq!(d.is_tinytex(), expect, "{name}");
        assert_eq!(d.kind == Kind::TinyTeX, expect, "{name} kind");
    }
}

#[test]
fn is_tinytex_by_marker_file() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("texlive");
    let d = Distribution::from_bin_dir(fake_install(&root));
    assert!(!d.is_tinytex());
    std::fs::write(root.join(".tinytex"), "").unwrap();
    assert!(d.is_tinytex());
}

#[test]
fn is_tinytex_by_fmtutil_cnf_mentioning_tinytex() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("texlive");
    let d = Distribution::from_bin_dir(fake_install(&root));
    let web2c = root.join("texmf-dist").join("web2c");
    std::fs::create_dir_all(&web2c).unwrap();
    std::fs::write(
        web2c.join("fmtutil.cnf"),
        "# /home/u/.TinyTeX/texmf-var\npdftex pdftex language.def\n",
    )
    .unwrap();
    assert!(d.is_tinytex());
    std::fs::write(web2c.join("fmtutil.cnf"), "pdftex pdftex language.def\n").unwrap();
    assert!(!d.is_tinytex());
}

#[test]
fn installed_version_reads_the_version_file() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("TinyTeX");
    let d = Distribution::from_bin_dir(fake_install(&root));
    assert_eq!(d.installed_version(), None);
    std::fs::write(root.join("version"), "v2026.01\n").unwrap();
    assert_eq!(d.installed_version().as_deref(), Some("v2026.01"));
}

// ---- default_install_dir ----

fn host(os: HostOs) -> HostEnv {
    HostEnv {
        os,
        home: Some(PathBuf::from("/home/u")),
        appdata: None,
        program_data: Some("C:\\ProgramData".into()),
    }
}

#[test]
fn default_install_dir_per_platform() {
    let c = Config::default();
    assert_eq!(
        default_install_dir_for(&c, &host(HostOs::Linux)),
        Some(PathBuf::from("/home/u/.TinyTeX"))
    );
    assert_eq!(
        default_install_dir_for(&c, &host(HostOs::MacOs)),
        Some(PathBuf::from("/home/u/Library/TinyTeX"))
    );
}

#[test]
fn default_install_dir_honors_config_install_dir() {
    let c = cfg_dir(Path::new("/elsewhere/TT"));
    for os in [HostOs::Linux, HostOs::MacOs, HostOs::Windows] {
        assert_eq!(
            default_install_dir_for(&c, &host(os)),
            Some(PathBuf::from("/elsewhere/TT"))
        );
    }
}

#[test]
fn windows_uses_appdata_unless_it_is_not_plain_ascii() {
    let c = Config::default();
    let mut h = host(HostOs::Windows);
    h.appdata = Some("C:\\Users\\bob\\AppData\\Roaming".into());
    assert_eq!(
        default_install_dir_for(&c, &h),
        Some(Path::new("C:\\Users\\bob\\AppData\\Roaming").join("TinyTeX"))
    );
    // Non-ASCII, and a space, both fall back to ProgramData.
    for bad in [
        "C:\\Users\\José\\AppData\\Roaming",
        "C:\\Users\\Bob Smith\\AppData\\Roaming",
    ] {
        h.appdata = Some(bad.into());
        assert_eq!(
            default_install_dir_for(&c, &h),
            Some(Path::new("C:\\ProgramData").join("TinyTeX")),
            "{bad}"
        );
    }
    h.appdata = None;
    assert_eq!(
        default_install_dir_for(&c, &h),
        Some(Path::new("C:\\ProgramData").join("TinyTeX"))
    );
    h.program_data = None;
    assert_eq!(default_install_dir_for(&c, &h), None);
}

// ---- find ----

#[test]
fn find_tinytex_is_filesystem_only_and_locates_the_bin_dir() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("TinyTeX");
    let bin = fake_install(&root);
    std::fs::create_dir_all(root.join("tlpkg")).unwrap();
    let d = tinytex::detect::find_tinytex(&cfg_dir(&root)).expect("found");
    assert_eq!(d.bin_dir, bin);
    assert_eq!(d.tlmgr, bin.join(tlmgr_name()));
    assert_eq!(d.root.as_deref(), Some(root.as_path()));
    assert_eq!(d.kind, Kind::TinyTeX);
    // No tlmgr, no distribution.
    assert!(tinytex::detect::find_tinytex(&cfg_dir(&tmp.path().join("nothing"))).is_none());
}

#[test]
fn find_with_prefer_tinytex_takes_the_tinytex_dir_then_path() {
    let tmp = TempDir::new().unwrap();
    let tt = tmp.path().join("TinyTeX");
    let tt_bin = fake_install(&tt);
    let sys = tmp.path().join("sys");
    let sys_bin = fake_install(&sys);
    let path = std::env::join_paths([&sys_bin]).unwrap();
    let h = host(HostOs::Linux);

    let mut c = cfg_dir(&tt);
    c.prefer = Prefer::TinyTeX;
    assert_eq!(find_with(&c, &h, Some(&path), &[]).unwrap().bin_dir, tt_bin);

    // TinyTeX dir absent: falls back to tlmgr on PATH, a plain TeX Live.
    let c2 = cfg_dir(&tmp.path().join("absent"));
    let d = find_with(&c2, &h, Some(&path), &[]).unwrap();
    assert_eq!(d.bin_dir, sys_bin);
    assert_eq!(d.kind, Kind::TexLive);
}

#[test]
fn find_with_prefer_system_takes_path_first() {
    let tmp = TempDir::new().unwrap();
    let tt = tmp.path().join("TinyTeX");
    let _ = fake_install(&tt);
    let sys = tmp.path().join("sys");
    let sys_bin = fake_install(&sys);
    let path = std::env::join_paths([&sys_bin]).unwrap();
    let mut c = cfg_dir(&tt);
    c.prefer = Prefer::System;
    let d = find_with(&c, &host(HostOs::Linux), Some(&path), &[]).unwrap();
    assert_eq!(d.bin_dir, sys_bin);
    // With nothing on PATH it still falls back to the TinyTeX directory.
    let d = find_with(&c, &host(HostOs::Linux), None, &[]).unwrap();
    assert_eq!(d.root.as_deref(), Some(tt.as_path()));
}

#[test]
fn find_with_checks_extra_directories_after_path() {
    let tmp = TempDir::new().unwrap();
    let texbin = tmp.path().join("texbin");
    std::fs::create_dir_all(&texbin).unwrap();
    let t = texbin.join(tlmgr_name());
    std::fs::write(&t, "").unwrap();
    make_executable(&t);
    let c = cfg_dir(&tmp.path().join("absent"));
    let d = find_with(
        &c,
        &host(HostOs::MacOs),
        None,
        std::slice::from_ref(&texbin),
    )
    .unwrap();
    assert_eq!(d.bin_dir, texbin);
    assert_eq!(d.root, None, "a system texbin has no discoverable root");
}

// ---- tex_live_year ----

struct Fake {
    calls: Mutex<Vec<CommandSpec>>,
    reply: io::Result<(i32, &'static str)>,
}

impl Exec for Fake {
    fn run(&self, cmd: &CommandSpec) -> io::Result<Output> {
        self.calls.lock().unwrap().push(cmd.clone());
        match &self.reply {
            Ok((code, text)) => Ok(Output {
                status: exit_status(*code),
                stdout: text.as_bytes().to_vec(),
                stderr: Vec::new(),
            }),
            Err(e) => Err(io::Error::new(e.kind(), "boom")),
        }
    }
}

fn year_of(reply: io::Result<(i32, &'static str)>) -> (Option<u32>, Vec<CommandSpec>) {
    let tmp = TempDir::new().unwrap();
    let d = Distribution::from_bin_dir(fake_install(&tmp.path().join("TinyTeX")));
    let f = Fake {
        calls: Mutex::new(Vec::new()),
        reply,
    };
    let y = d.tex_live_year(&f);
    (y, f.calls.into_inner().unwrap())
}

#[test]
fn tex_live_year_parses_the_version_line_through_exec() {
    let text = "tlmgr revision 76773 (2025-11-06 20:43:29 +0100)\ntlmgr using installation: /x\nTeX Live (https://tug.org/texlive) version 2025\n";
    let (y, calls) = year_of(Ok((0, text)));
    assert_eq!(y, Some(2025));
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].args, [OsString::from("--version")]);
    assert!(
        calls[0]
            .program
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("tlmgr")
    );
    assert!(!calls[0].changes_installation);
    assert_eq!(
        year_of(Ok((0, "TeX Live (https://tug.org/texlive) version 2025"))).0,
        Some(2025)
    );
}

#[test]
fn tex_live_year_is_none_for_empty_garbled_or_failing_output() {
    assert_eq!(year_of(Ok((0, ""))).0, None);
    assert_eq!(year_of(Ok((0, "TeX Live version soon"))).0, None);
    assert_eq!(
        year_of(Ok((1, "TeX Live (https://tug.org/texlive) version 2025"))).0,
        None
    );
    assert_eq!(year_of(Err(io::Error::other("x"))).0, None);
}

#[test]
fn parse_tl_version_reads_year_and_date() {
    let text = "tlmgr revision 76773 (2025-11-06 20:43:29 +0100)\nTeX Live (https://tug.org/texlive) version 2025\n";
    let v = tinytex::detect::parse_tl_version(text).unwrap();
    assert_eq!((v.year, v.date.as_str()), (2025, "2025-11-06"));
    let v = tinytex::detect::parse_tl_version("TeX Live (https://tug.org/texlive) version 2023")
        .unwrap();
    assert_eq!((v.year, v.date.as_str()), (2023, ""));
}

// ---- is_writable ----

#[test]
fn is_writable_needs_tinytex_and_a_passing_probe() {
    let tmp = TempDir::new().unwrap();
    let tt = Distribution::from_bin_dir(fake_install(&tmp.path().join("TinyTeX")));
    let sys = Distribution::from_bin_dir(fake_install(&tmp.path().join("texlive")));
    let probed = RefCell::new(Vec::new());
    let yes = |p: &Path| {
        probed.borrow_mut().push(p.to_path_buf());
        true
    };
    assert!(tt.is_writable_with(&yes));
    assert_eq!(probed.borrow().as_slice(), std::slice::from_ref(&tt.tlmgr));
    assert!(!tt.is_writable_with(&|_| false));
    assert!(!sys.is_writable_with(&|_| true), "not TinyTeX");
}

#[test]
fn is_writable_real_probe_checks_write_access_not_metadata() {
    let tmp = TempDir::new().unwrap();
    let d = Distribution::from_bin_dir(fake_install(&tmp.path().join("TinyTeX")));
    assert!(d.is_writable());
    let missing = Distribution {
        tlmgr: tmp.path().join("nope"),
        ..d
    };
    assert!(!missing.is_writable());
}
