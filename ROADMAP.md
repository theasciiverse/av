# Roadmap

av is at 0.8.0 (2026-10-28): the whole first pass works on Linux, from the
catalog to playing a title. The welcome screen, the intro and news are still
to come, and it has never run on macOS or Windows. 1.0.0 is when everything
below is in and av has had its first public release.

## 0.8.0: built

- [x] The catalog: fetched, cached, refreshed with F5, its schema version
  checked
- [x] One list for catalog and library, with search and the show card;
  installed, updates and wishlisted in the menu row
- [x] The detail pane: the live `.cast` preview, the store page in Markdown,
  the facts, the minimum size and system package warnings
- [x] Install, update and uninstall, every download checked against the
  catalog's sha256; `.tar.gz` and `.zip`; links in `~/.local/bin` and the
  PATH hint
- [x] Launching titles, with a log per title; MUDs in Lera, or telnet
- [x] The downloads pane: a queue, retry, cancel, updates offered and only
  started on a yes
- [x] The shell commands (`install`, `update`, `uninstall`, `play`, `list`,
  `search`, `info`) and tab completion for bash, zsh, fish and PowerShell
- [x] Wishlist, help, ♥ support, session logging
- [x] CI and the release workflow for six targets; licence MIT

## 0.9.0: the first run

- [ ] The welcome screen, shown once and again from help: what av is, the
  PATH check, the note on fonts with Braille and block characters
- [ ] The intro: the line growing in from both edges, then the ASCIIVERSE
  lettering
- [ ] News: a title's release notes and CHANGELOG in its page
- [ ] Tests for install, update and uninstall against a local catalog,
  including a download whose checksum doesn't match

## 1.0.0

- [ ] Run and checked on macOS and Windows, and CI passing on all three
- [ ] The installers' address, `asciiverse.sh/releases/av/latest`, sent on
  to GitHub Releases (a `_redirects` line in `hub`)
- [ ] The first public release, against the real catalog at asciiverse.sh

## After 1.0

Not decided yet (see `CLAUDE.md`):

- [ ] Recording your own `.cast` while playing
- [ ] "Write a review" as a link to a pre-filled GitHub Discussion
- [ ] More than one library location
- [ ] Launching movies
- [ ] Paid titles
