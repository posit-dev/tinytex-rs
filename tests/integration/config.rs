use std::cell::RefCell;
use std::ffi::OsString;
use std::path::PathBuf;

use tinytex::{Config, Prefer, TlmgrUpdate};

#[test]
fn default_is_auto_update_prefer_tinytex_and_otherwise_empty() {
    let c = Config::default();
    assert_eq!(c.tlmgr_update, TlmgrUpdate::Auto);
    assert_eq!(c.prefer, Prefer::TinyTeX);
    assert_eq!(c.install_dir, None);
    assert_eq!(c.repository, None);
    assert!(c.tlmgr_opts.is_empty());
    assert!(!c.prevent_install);
    assert_eq!(c.tlmgr_path, None);
    assert!(!c.usermode && !c.delete_tlpdb && !c.verbose);
}

#[test]
fn from_env_reads_tinytex_dir_and_prevent_install_and_nothing_else() {
    let asked = RefCell::new(Vec::new());
    let c = Config::from_env_with(|name| {
        asked.borrow_mut().push(name.to_owned());
        match name {
            "TINYTEX_DIR" => Some(OsString::from("/opt/tt")),
            "TINYTEX_PREVENT_INSTALL" => Some(OsString::from("TRUE")),
            _ => Some(OsString::from("surprise")),
        }
    });
    assert_eq!(c.install_dir, Some(PathBuf::from("/opt/tt")));
    assert!(c.prevent_install);
    let mut asked = asked.into_inner();
    asked.sort();
    assert_eq!(asked, ["TINYTEX_DIR", "TINYTEX_PREVENT_INSTALL"]);
    assert_eq!(c.repository, None);
}

#[test]
fn from_env_treats_empty_dir_and_other_prevent_values_as_unset() {
    let c = Config::from_env_with(|name| match name {
        "TINYTEX_DIR" => Some(OsString::new()),
        "TINYTEX_PREVENT_INSTALL" => Some(OsString::from("no")),
        _ => None,
    });
    assert_eq!(c.install_dir, None);
    assert!(!c.prevent_install);
    assert_eq!(Config::from_env_with(|_| None), Config::default());
}

#[test]
fn config_round_trips_as_json() {
    let mut c = Config::default();
    c.install_dir = Some(PathBuf::from("/x"));
    c.tlmgr_opts = vec!["--repository".into(), "https://r".into()];
    c.prefer = Prefer::System;
    c.tlmgr_update = TlmgrUpdate::Never;
    c.usermode = true;
    let json = serde_json::to_string(&c).unwrap();
    assert!(
        json.contains("\"system\"") && json.contains("\"never\""),
        "{json}"
    );
    assert_eq!(serde_json::from_str::<Config>(&json).unwrap(), c);
}
