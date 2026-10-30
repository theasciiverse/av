# av

The [Asciiverse](https://asciiverse.sh) client: the whole catalog of
text-mode games, programs, text adventures, art and MUDs in your terminal.
Browse it, watch each title's preview, then install, update and launch, on
Linux, macOS and Windows, in whatever terminal you already use.

## Install

Linux and macOS:

```sh
curl -fsSL https://asciiverse.sh/install.sh | sh
```

Windows (PowerShell):

```powershell
irm https://asciiverse.sh/install.ps1 | iex
```

Both download the build for your system from this repository's releases,
check it against `SHA256SUMS` and put `av` on your PATH. To do it by hand,
take the archive for your system from the
[latest release](https://github.com/theasciiverse/av/releases/latest) and
put `av` (Windows: `av.exe`) somewhere on your PATH.

Debian and Ubuntu: the install line above offers the `.deb`, or take
`av_amd64.deb` (or `av_arm64.deb`) from the latest release and run
`sudo apt install ./av_amd64.deb`. It also adds Asciiverse to the
applications menu and sets up tab completion for bash, zsh and fish.

From source, with Rust 1.88 or newer:

```sh
cargo install --locked --git https://github.com/theasciiverse/av
```

## Use

`av` alone opens the catalog and your library. With a command it does one
thing and exits:

```text
av install <title>...     install titles, or update them to the latest version
av update [<title>...]    update these titles, or every title with an update
av uninstall <title>...   remove titles; their saves and settings are kept
av play <title> [args]    start a title (for a MUD: connect to it)
av list [installed|updates|wishlist]
av search <words>         find titles by name, topic, genre or developer
av info <title>           show a title's details
```

A title is its id (`av install solitty`) or its full name.

In the catalog: arrow keys move, `Tab` switches between the list and the
detail pane, `Enter` selects, typing filters, and `Esc` backs out one step
at a time. The card under the search box picks what the list shows: `↑`
from the top of the list opens it, `Space` switches a group (games, text
adventures, MUDs, programs, art and demos, movies) on or off, and two more
boxes narrow the list to what you have installed, what has an update, or
what you wishlisted. av remembers the choice. The counts at the top do the
same in one click, and `F1` shows all the keys.

`F2` opens the downloads: installs and updates run one at a time with the
rest queued, and a failed one can be retried. Underneath, it offers the
updates waiting, one per row: Enter says yes to one, `u` to all of them.
Nothing updates unless you say so.

The terminal should be at least 128x40 to show a title's preview in full;
smaller terminals crop it. Some titles draw with Braille or Unicode block
characters and need a font that has them.

### Tab completion

`av completions <shell>` prints completion for commands and title ids:
`av install` offers the whole catalog, `av play` what is installed plus the
MUDs. It answers from the cached catalog, so it is instant and works
offline. Add the line for your shell:

| Shell | Where | Line |
|---|---|---|
| bash | `~/.bashrc` | `eval "$(av completions bash)"` |
| zsh | `~/.zshrc`, after `compinit` | `source <(av completions zsh)` |
| fish | run once | `av completions fish > ~/.config/fish/completions/av.fish` |
| PowerShell | `$PROFILE` | `av completions powershell \| Out-String \| Invoke-Expression` |

## How it works

There is no server and no account. `av` reads a static `catalog.json` that
the catalog repository publishes, and caches it, so the library works
offline. Every download is checked against the sha256 pinned in the reviewed
catalog before anything is unpacked, and updates only ever happen when you
start them. Each title runs as its own program; `av` only starts it.

Where things go:

| | Linux | macOS | Windows |
|---|---|---|---|
| titles | `~/.local/share/asciiverse/` | `~/Library/Application Support/Asciiverse/` | `%LOCALAPPDATA%\Asciiverse\` |
| cache, log | `~/.cache/asciiverse/` | `~/Library/Caches/Asciiverse/` | `%LOCALAPPDATA%\Asciiverse\cache\` |

On Linux and macOS each installed title is also linked into `~/.local/bin`,
so it can be started from the shell by its id. The cache is always safe to
delete.

`AV_CATALOG_URL` points `av` at another catalog, and `RUST_LOG=av=debug`
writes a detailed log to `av.log` in the cache folder.

## Adding a title

Titles join the catalog by pull request to the catalog repository, open or
closed source alike; see its `CONTRIBUTING.md`.

## License

MIT, see [LICENSE](LICENSE). The titles `av` installs are separate programs
under their own licenses. The open-source crates `av` is built from are
listed with their licenses in `THIRD-PARTY-LICENSES.txt`, shipped in every
release archive.
