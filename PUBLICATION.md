# What this repository is, and what was excluded

This is the public source repository for **Looking Glass** — an unofficial Rust
re-implementation of *American McGee's Alice* — and for the **LookingGlassNX
Nintendo Switch port** on branch `switch`.

It is **source code only**. It is published from a private development
repository after a history filter, and this file records exactly what that
filter removed and why, so nobody has to guess what is missing.

## Branches

| branch | contents |
|---|---|
| `main` | the engine, with the same source as the upstream Looking Glass project |
| `switch` | the engine plus the LookingGlassNX Switch port (devkitA64/libnx/EGL/miniquad Horizon backend, the Alice asset pipeline, and the hardware bring-up work) |

Tags are intentionally **not** published: upstream releases and downloads live at
[looking-glass-project/LookingGlass](https://github.com/looking-glass-project/LookingGlass).
This repository is a source mirror plus the Switch port, not a release channel.

## Excluded content

Nothing below is present in any commit on any branch, including rewritten
history.

| excluded | size | why |
|---|---|---|
| `*.elf`, `*.nro`, `*.nacp` build outputs | 41 MB + 29 MB per build | build artifacts; regenerable, and too large for source history |
| `docs/media/gameplay/*` (`.mp4`, `.jpg`, `.gif`) | 9 MB | gameplay captures contain original game content, which is EA's |
| `switch/**/assets/*-icon-source.png`, `switch/**/icon.jpg` | 2.4 MB | derived from game art; the shipped icon is regenerated locally |
| any original game data (`*.pk3`, `*.tga`, `*.ftx`, `*.pcx`, `*.cik`, audio) | — | never committed; the engine reads the user's own files at runtime |
| all other binary media (`png/jpg/gif/mp4/wav/mp3/ogg/zip/7z/bin`) | — | the repository publishes source, not media |

`THIRD_PARTY_NOTICES.txt`, `LICENSE-MIT` and `LEGAL.md` are kept verbatim.
`LEGAL.md` states the project's position on EA's ownership of the original game,
and it applies unchanged to this repository.

## How the history was filtered

All commits were rewritten with `git filter-branch --prune-empty
--index-filter` using this rule, applied to every commit on every published
branch:

```sh
git ls-files -z \
  | grep -zE '\.(elf|nro|nacp|pk3|tga|ftx|pcx|cik|mp4|gif|jpe?g|png|wav|mp3|ogg|zip|7z|bin)$' \
  | xargs -0 -r git rm --cached -q --ignore-unmatch
git rm -r --cached -q --ignore-unmatch docs/media/gameplay
```

Consequences, stated plainly:

- **Commit hashes differ from the private repository and from upstream.** The
  content of each commit is otherwise the same commit it was, and
  **authorship is preserved**: author and committer names, emails and dates are
  untouched by the filter.
- Commits that consisted only of excluded files were pruned, so the commit count
  can be lower than in the private repository.
- Because hashes are rewritten, this repository cannot be merged from upstream
  with a plain `git pull`. Fetch upstream and cherry-pick or merge by content.

The filter is reproducible: run the two commands above on a fresh clone of the
private repository, then `git gc --prune=now`, and the result matches this
repository's history modulo the documentation commits that follow it.

## Working on it

See the README for build instructions, including the pinned nightly toolchain and
the devkitA64 builder image used for the Switch NRO. Contributors should read
`CONTRIBUTING.md` first: the source boundary there is the same rule this
publication follows.

You need your own compatible game files to run anything. None are provided, and
none will be accepted into this repository.
