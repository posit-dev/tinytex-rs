use tinytex::log::SearchTerm;
use tinytex::{Event, NullObserver, Observer};

fn all() -> Vec<Event> {
    vec![
        Event::Searching {
            term: SearchTerm::File("/pgfplots.sty".into()),
        },
        Event::InstallingPackage {
            name: "pgf".into(),
            index: 1,
            of: 3,
        },
        Event::UpdatingTlmgr,
        Event::UpdatingPackages,
        Event::RebuildingFormats,
        Event::RunningEngine { run: 2 },
        Event::MakingIndex {
            command: "makeindex".into(),
        },
        Event::RunningBibEngine {
            engine: "biber".into(),
        },
        Event::InstallingHyphenation {
            file: "hyph-de.tex".into(),
        },
        Event::PackagesInstalled {
            names: vec!["a".into(), "b".into()],
        },
        Event::Downloading {
            url: "https://x/y".into(),
            received: 5,
            total: Some(10),
        },
        Event::Downloading {
            url: "https://x/y".into(),
            received: 5,
            total: None,
        },
    ]
}

#[test]
fn every_variant_round_trips_as_json() {
    for e in all() {
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Event>(&json).unwrap(), e, "{json}");
    }
}

#[test]
fn every_variant_has_a_nonempty_display_text() {
    for e in all() {
        assert!(!e.to_string().trim().is_empty(), "{e:?}");
    }
    assert!(
        Event::InstallingPackage {
            name: "pgf".into(),
            index: 1,
            of: 3
        }
        .to_string()
        .contains("pgf")
    );
    assert!(
        Event::PackagesInstalled {
            names: vec!["a".into(), "b".into()]
        }
        .to_string()
        .contains("a, b")
    );
}

#[test]
fn null_observer_is_a_no_op() {
    let o: &dyn Observer = &NullObserver;
    for e in all() {
        o.event(&e);
    }
}
