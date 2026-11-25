//! The fail-closed gate of `sandbox`: every refusal has its own test in which
//! every other check is satisfied, so a gate that skips one check fails that
//! test. Unix only (the fake trees hold symlinks and shell scripts); a
//! non-Unix build refuses everything, which the last test pins.

use super::sandbox::{Inputs, Refusal, check};

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;
    use tinytex::{CommandSpec, Exec};

    use super::{Inputs, Refusal, check};

    const NONCE: &str = "nonce-1";

    fn script(path: &Path, body: &str) {
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A kpsewhich that prints the root it lives in (shell builtins only: the
    /// child `PATH` holds no other tools).
    const KPSEWHICH_OWN_ROOT: &str = r#"cd "${0%/*}/../.." && pwd -P"#;

    /// A valid fake sandbox under a fake temporary directory, plus the inputs
    /// that accept it. A test breaks exactly one thing.
    struct Fixture {
        /// Holds `tmp`, the fake default install directory, and the system dir.
        _keep: TempDir,
        tmp: PathBuf,
        root: PathBuf,
        other: PathBuf,
        system: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let keep = tempfile::tempdir().unwrap();
            let top = keep.path().canonicalize().unwrap();
            let tmp = top.join("tmp");
            let other = top.join("other");
            let system = top.join("system");
            for d in [&tmp, &other, &system] {
                std::fs::create_dir(d).unwrap();
            }
            let root = tmp.join("tinytex-sandbox.test01");
            let bin = root.join("bin/arch");
            std::fs::create_dir_all(&bin).unwrap();
            script(&bin.join("tlmgr"), "exit 0");
            script(&bin.join("kpsewhich"), KPSEWHICH_OWN_ROOT);
            std::fs::write(
                root.join("DISPOSABLE_TINYTEX"),
                format!("{}\n{NONCE}\n", root.display()),
            )
            .unwrap();
            std::fs::write(root.join("COMPLETE"), "").unwrap();
            Self {
                _keep: keep,
                tmp,
                root,
                other,
                system,
            }
        }

        fn inputs(&self) -> Inputs {
            Inputs {
                root: Some(self.root.clone()),
                nonce: Some(NONCE.to_owned()),
                temp_dirs: vec![self.tmp.clone()],
                default_dirs: vec![self.other.join(".TinyTeX")],
                system_path: vec![self.system.clone()],
            }
        }

        fn bin(&self) -> PathBuf {
            self.root.join("bin/arch")
        }
    }

    fn refused(i: &Inputs) -> Refusal {
        match check(i) {
            Ok(_) => panic!("the gate accepted inputs that must be refused: {i:?}"),
            Err(r) => r,
        }
    }

    #[test]
    fn a_complete_sandbox_is_accepted() {
        let f = Fixture::new();
        let sb = check(&f.inputs()).unwrap_or_else(|r| panic!("refused: {r}"));
        assert_eq!(sb.root, f.root);
        assert_eq!(sb.home, PathBuf::from(format!("{}.home", f.root.display())));
        assert!(
            sb.home.is_dir(),
            "the gate creates the sibling home directory"
        );
        assert_eq!(sb.dist.root.as_deref(), Some(f.root.as_path()));
    }

    #[test]
    fn refuses_when_no_root_is_given() {
        let f = Fixture::new();
        let mut i = f.inputs();
        i.root = None;
        assert_eq!(refused(&i), Refusal::RootUnset);
    }

    #[test]
    fn refuses_a_root_that_does_not_exist() {
        let f = Fixture::new();
        let mut i = f.inputs();
        i.root = Some(f.tmp.join("tinytex-sandbox.typo"));
        assert!(matches!(refused(&i), Refusal::RootUnreadable(_)));
    }

    #[test]
    fn refuses_the_default_install_directory_itself() {
        let f = Fixture::new();
        let mut i = f.inputs();
        i.default_dirs = vec![f.root.clone()];
        assert!(matches!(refused(&i), Refusal::DefaultInstallDir(_)));
    }

    #[test]
    fn refuses_a_root_inside_a_default_install_directory() {
        let f = Fixture::new();
        let mut i = f.inputs();
        i.default_dirs = vec![f.tmp.clone()];
        assert!(matches!(refused(&i), Refusal::DefaultInstallDir(_)));
    }

    #[test]
    fn refuses_a_default_install_directory_named_through_a_symlink() {
        // A lexical comparison is defeated by a symlinked home.
        let f = Fixture::new();
        let link = f.other.join("home-link");
        symlink(&f.root, &link).unwrap();
        let mut i = f.inputs();
        i.default_dirs = vec![link];
        assert!(matches!(refused(&i), Refusal::DefaultInstallDir(_)));
    }

    #[test]
    fn refuses_a_root_outside_the_temporary_directories() {
        let f = Fixture::new();
        let mut i = f.inputs();
        i.temp_dirs = vec![f.other.clone()];
        assert!(matches!(refused(&i), Refusal::NotUnderTemp(_)));
    }

    #[test]
    fn a_root_inside_the_second_temporary_directory_is_accepted() {
        // $RUNNER_TEMP is a second allowed base.
        let f = Fixture::new();
        let mut i = f.inputs();
        i.temp_dirs = vec![f.other.clone(), f.tmp.clone()];
        check(&i).unwrap_or_else(|r| panic!("refused: {r}"));
    }

    #[test]
    fn refuses_a_root_without_the_sentinel() {
        let f = Fixture::new();
        std::fs::remove_file(f.root.join("DISPOSABLE_TINYTEX")).unwrap();
        assert_eq!(refused(&f.inputs()), Refusal::SentinelMissing);
    }

    #[test]
    fn refuses_a_sentinel_that_names_another_path() {
        // What a copy of a sandbox looks like.
        let f = Fixture::new();
        std::fs::write(
            f.root.join("DISPOSABLE_TINYTEX"),
            format!("/somewhere/else\n{NONCE}\n"),
        )
        .unwrap();
        assert!(matches!(refused(&f.inputs()), Refusal::SentinelPath { .. }));
    }

    #[test]
    fn refuses_another_runs_nonce() {
        let f = Fixture::new();
        let mut i = f.inputs();
        i.nonce = Some("a-different-nonce".to_owned());
        assert_eq!(refused(&i), Refusal::NonceMismatch);
    }

    #[test]
    fn refuses_when_the_nonce_is_not_exported() {
        let f = Fixture::new();
        let mut i = f.inputs();
        i.nonce = None;
        assert_eq!(refused(&i), Refusal::NonceMismatch);
    }

    #[test]
    fn refuses_a_copy_that_did_not_finish() {
        let f = Fixture::new();
        std::fs::remove_file(f.root.join("COMPLETE")).unwrap();
        assert_eq!(refused(&f.inputs()), Refusal::Incomplete);
    }

    #[test]
    fn refuses_a_root_without_a_tlmgr_and_does_not_fall_back_to_the_system_path() {
        let f = Fixture::new();
        std::fs::remove_file(f.bin().join("tlmgr")).unwrap();
        assert_eq!(refused(&f.inputs()), Refusal::NoDistribution);
    }

    #[test]
    fn refuses_a_tlmgr_whose_real_path_leaves_the_root() {
        let f = Fixture::new();
        let outside = f.other.join("tlmgr.pl");
        script(&outside, "exit 0");
        std::fs::remove_file(f.bin().join("tlmgr")).unwrap();
        symlink(&outside, f.bin().join("tlmgr")).unwrap();
        assert!(matches!(refused(&f.inputs()), Refusal::EscapesRoot { .. }));
    }

    #[test]
    fn refuses_a_kpsewhich_whose_real_path_leaves_the_root() {
        let f = Fixture::new();
        let outside = f.other.join("kpsewhich");
        // It would even print the right root: only the path check catches it.
        script(&outside, &format!("echo {}", f.root.display()));
        std::fs::remove_file(f.bin().join("kpsewhich")).unwrap();
        symlink(&outside, f.bin().join("kpsewhich")).unwrap();
        assert!(matches!(refused(&f.inputs()), Refusal::EscapesRoot { .. }));
    }

    #[test]
    fn refuses_a_system_directory_that_holds_a_tex_tool() {
        for tool in ["tlmgr", "kpsewhich", "pdflatex"] {
            let f = Fixture::new();
            script(&f.system.join(tool), "exit 0");
            assert!(
                matches!(refused(&f.inputs()), Refusal::SystemPathHasTex(_)),
                "{tool}"
            );
        }
    }

    #[test]
    fn refuses_a_kpsewhich_that_prints_another_root() {
        let f = Fixture::new();
        script(
            &f.bin().join("kpsewhich"),
            "echo /Users/someone/Library/TinyTeX",
        );
        assert!(matches!(
            refused(&f.inputs()),
            Refusal::KpsewhichRoot { .. }
        ));
    }

    #[test]
    fn refuses_a_kpsewhich_that_fails() {
        let f = Fixture::new();
        script(&f.bin().join("kpsewhich"), "exit 3");
        assert!(matches!(
            refused(&f.inputs()),
            Refusal::KpsewhichRoot { .. }
        ));
    }

    #[test]
    fn the_sandbox_exec_clears_the_environment_and_forces_the_paths() {
        let f = Fixture::new();
        let sb = check(&f.inputs()).unwrap();
        let mut spec = CommandSpec::new("/usr/bin/env");
        spec.env.push(("PATH".into(), "/evil/bin".into()));
        spec.env.push(("HOME".into(), "/Users/someone".into()));
        spec.env
            .push(("TEXMFROOT".into(), "/Users/someone/Library/TinyTeX".into()));
        spec.env.push(("KEEP_ME".into(), "yes".into()));
        let out = sb.exec().run(&spec).unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let vars: std::collections::BTreeMap<&str, &str> =
            text.lines().filter_map(|l| l.split_once('=')).collect();

        assert_eq!(vars.get("HOME").copied(), Some(sb.home.to_str().unwrap()));
        let path = vars.get("PATH").copied().unwrap();
        assert!(path.starts_with(f.bin().to_str().unwrap()), "{path}");
        assert!(!path.contains("/evil"), "{path}");
        assert!(!vars.contains_key("TEXMFROOT"), "{text}");
        assert_eq!(vars.get("KEEP_ME").copied(), Some("yes"));
        // The parent's own variables do not reach the child.
        assert!(!vars.contains_key("CARGO_MANIFEST_DIR"), "{text}");
        assert!(!vars.contains_key("CARGO"), "{text}");
    }
}

#[cfg(not(unix))]
#[test]
fn a_non_unix_build_refuses_everything() {
    let i = Inputs {
        root: None,
        nonce: None,
        temp_dirs: vec![],
        default_dirs: vec![],
        system_path: vec![],
    };
    assert_eq!(check(&i).err(), Some(Refusal::Unsupported));
}
