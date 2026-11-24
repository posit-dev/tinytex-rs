//! `scripts/tex-sandbox.sh`: builds, deletes, and cleans disposable copies of
//! a TeX tree. Unix only: the script is POSIX `sh` and the copies hold
//! symlinks.
#![cfg(unix)]

use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts/tex-sandbox.sh")
}

/// Runs the script with `base` as the temporary directory.
fn run(base: &Path, args: &[&str]) -> Output {
    Command::new("sh")
        .arg(script())
        .args(args)
        .env("TMPDIR", base)
        .env_remove("RUNNER_TEMP")
        .output()
        .expect("run tex-sandbox.sh")
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// A small tree shaped like TinyTeX: `bin/arch/tlmgr` is a relative symlink
/// into `texmf-dist`, and the two directories the script strips exist.
fn source() -> TempDir {
    let src = tempfile::tempdir().unwrap();
    let r = src.path();
    std::fs::create_dir_all(r.join("bin/arch")).unwrap();
    std::fs::create_dir_all(r.join("texmf-dist/scripts")).unwrap();
    std::fs::write(r.join("texmf-dist/scripts/tlmgr.pl"), "#!/bin/sh\n").unwrap();
    symlink("../../texmf-dist/scripts/tlmgr.pl", r.join("bin/arch/tlmgr")).unwrap();
    std::fs::create_dir_all(r.join("texmf-var/fonts/conf")).unwrap();
    std::fs::write(r.join("texmf-var/fonts/conf/leak.conf"), "/original/fonts").unwrap();
    std::fs::create_dir_all(r.join("texmf-var/luatex-cache")).unwrap();
    std::fs::write(r.join("texmf-var/keep.txt"), "kept").unwrap();
    src
}

fn var(out: &Output, name: &str) -> String {
    text(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name}=")).map(str::to_owned))
        .unwrap_or_else(|| panic!("no {name}= in {:?}", text(&out.stdout)))
}

fn create(base: &Path, src: &Path) -> (PathBuf, String) {
    let out = run(base, &["create", src.to_str().unwrap()]);
    assert!(out.status.success(), "create failed: {}", text(&out.stderr));
    (PathBuf::from(var(&out, "TINYTEX_DIR")), var(&out, "TINYTEX_SANDBOX_NONCE"))
}

#[test]
fn create_copies_the_tree_and_writes_the_sentinel_then_the_marker() {
    let base = tempfile::tempdir().unwrap();
    let src = source();
    let (root, nonce) = create(base.path(), src.path());

    assert_eq!(root.parent().unwrap(), base.path().canonicalize().unwrap());
    assert!(root.file_name().unwrap().to_string_lossy().starts_with("tinytex-sandbox."));
    assert!(!nonce.is_empty());
    let sentinel = std::fs::read_to_string(root.join("DISPOSABLE_TINYTEX")).unwrap();
    assert_eq!(sentinel, format!("{}\n{}\n", root.display(), nonce));
    assert!(root.join("COMPLETE").exists());
    assert_eq!(std::fs::read_to_string(root.join("texmf-var/keep.txt")).unwrap(), "kept");
    // Symlinks are copied as symlinks, not followed.
    let link = root.join("bin/arch/tlmgr");
    assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
    assert_eq!(
        std::fs::read_link(&link).unwrap(),
        Path::new("../../texmf-dist/scripts/tlmgr.pl")
    );
    // The two directories that point at the original are gone.
    assert!(!root.join("texmf-var/fonts/conf").exists());
    assert!(!root.join("texmf-var/luatex-cache").exists());
    // The sibling home directory exists, and the source is untouched.
    let home = PathBuf::from(format!("{}.home", root.display()));
    assert!(home.is_dir(), "{} missing", home.display());
    assert!(src.path().join("texmf-var/fonts/conf/leak.conf").exists());
}

#[test]
fn create_refuses_an_absolute_symlink_and_leaves_nothing_behind() {
    let base = tempfile::tempdir().unwrap();
    let src = source();
    symlink("/usr/bin", src.path().join("texmf-dist/abs")).unwrap();
    let out = run(base.path(), &["create", src.path().to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("absolute symlink"), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_dir(base.path()).unwrap().count(), 0);
}

#[test]
fn create_refuses_a_relative_symlink_that_escapes_the_tree() {
    let base = tempfile::tempdir().unwrap();
    let src = source();
    symlink("../../../outside", src.path().join("texmf-dist/up")).unwrap();
    let out = run(base.path(), &["create", src.path().to_str().unwrap()]);
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("escapes"), "{}", text(&out.stderr));
    assert_eq!(std::fs::read_dir(base.path()).unwrap().count(), 0);
}

#[test]
fn create_refuses_a_source_that_is_itself_a_sandbox() {
    let base = tempfile::tempdir().unwrap();
    let src = source();
    std::fs::write(src.path().join("DISPOSABLE_TINYTEX"), "x\ny\n").unwrap();
    let out = run(base.path(), &["create", src.path().to_str().unwrap()]);
    assert!(!out.status.success());
}

#[test]
fn delete_needs_this_runs_nonce_and_removes_the_tree_and_its_home() {
    let base = tempfile::tempdir().unwrap();
    let src = source();
    let (root, nonce) = create(base.path(), src.path());
    let home = PathBuf::from(format!("{}.home", root.display()));

    let wrong = run(base.path(), &["delete", root.to_str().unwrap(), "not-the-nonce"]);
    assert!(!wrong.status.success());
    assert!(root.exists(), "a wrong nonce must delete nothing");

    let ok = run(base.path(), &["delete", root.to_str().unwrap(), &nonce]);
    assert!(ok.status.success(), "{}", text(&ok.stderr));
    assert!(!root.exists() && !home.exists());
}

#[test]
fn delete_refuses_a_directory_outside_the_base_or_without_the_prefix() {
    let base = tempfile::tempdir().unwrap();
    // Has a sentinel with a nonce, but its name has no prefix.
    let plain = base.path().join("some-other-dir");
    std::fs::create_dir(&plain).unwrap();
    std::fs::write(plain.join("DISPOSABLE_TINYTEX"), "p\nn\n").unwrap();
    assert!(!run(base.path(), &["delete", plain.to_str().unwrap(), "n"]).status.success());
    assert!(plain.exists());

    // Prefixed and sentineled, but in another directory than the base.
    let elsewhere = tempfile::tempdir().unwrap();
    let far = elsewhere.path().join("tinytex-sandbox.zzzzzz");
    std::fs::create_dir(&far).unwrap();
    std::fs::write(far.join("DISPOSABLE_TINYTEX"), "p\nn\n").unwrap();
    assert!(!run(base.path(), &["delete", far.to_str().unwrap(), "n"]).status.success());
    assert!(far.exists());
}

#[test]
fn delete_refuses_a_symlink_with_the_prefix() {
    let base = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    std::fs::write(target.path().join("DISPOSABLE_TINYTEX"), "p\nn\n").unwrap();
    std::fs::write(target.path().join("precious"), "data").unwrap();
    let link = base.path().join("tinytex-sandbox.linked");
    symlink(target.path(), &link).unwrap();
    assert!(!run(base.path(), &["delete", link.to_str().unwrap(), "n"]).status.success());
    assert!(target.path().join("precious").exists());
}

#[test]
fn clean_removes_a_half_copied_tree_and_leaves_one_without_a_sentinel() {
    let base = tempfile::tempdir().unwrap();
    // What a crash between the sentinel and the marker leaves behind.
    let half = base.path().join("tinytex-sandbox.half123");
    std::fs::create_dir(&half).unwrap();
    std::fs::write(half.join("DISPOSABLE_TINYTEX"), "p\nn\n").unwrap();
    std::fs::create_dir(format!("{}.home", half.display())).unwrap();
    // Carries the prefix but no sentinel: not ours to delete.
    let foreign = base.path().join("tinytex-sandbox.foreign");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::write(foreign.join("data"), "x").unwrap();

    let out = run(base.path(), &["clean"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(!half.exists());
    assert!(!PathBuf::from(format!("{}.home", half.display())).exists());
    assert!(foreign.join("data").exists());
}
