# Changelog

## Unreleased

The first tag is a `0.x` pre-release. The API is not frozen: the first
consumers may find problems, which are fixed here and tagged.

Contents: log parsing, `Exec` and `Observer`, distribution detection, `tlmgr`,
and the compile driver with its engines, cleanup and output placement, and
the `tinytex` command-line binary (`cli` feature): `compile`, `detect`,
`tlmgr` and `log`, with a versioned JSON contract.

### Fixed

- `Tlmgr::install` no longer fails when none of the requested packages is installed. The
  installed-check, `tlmgr info --only-installed`, exits 1 with no output for an unknown
  package, and `Tlmgr::info` treated that as a command failure although its documentation
  says an unknown package only leaves its row out. `info` now returns an empty list when
  `tlmgr` says only that the packages are unknown, and still returns an error for any other
  complaint. The mocked `tlmgr` in the tests now answers like the real one, which is why the
  install tests had not caught this.
