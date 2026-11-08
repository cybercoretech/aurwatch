# 📦 aurwatch

Track your AUR packages 🐧: votes ⭐, popularity 📈, downloads ⬇️ and views 👀,
in a terminal UI 🖥️ and an Omarchy bar widget 🧩.

## 🚀 Install

```sh
./install.sh --enable
```

This installs the binary to `~/.local/bin/aurwatch`, enables a 6-hourly fetch
timer, and adds the bar widget (`custom.aurwatch`) to the bar's right section.
Without `--enable` the widget is installed but not added to the bar;
`--no-timer` skips the timer. Rerun it after pulling changes.

```sh
aurwatch                  # terminal UI
aurwatch fetch            # fetch now and print a table
aurwatch fetch --json     # full snapshot
aurwatch fetch --no-github
aurwatch summary          # compact JSON digest (what the bar widget reads)
aurwatch config           # path to config.toml (created on first run)
```

## 🧩 Bar widget

Shows the Arch logo and the total downloads of your `-bin` packages. It turns
the urgent color when a package needs attention (flagged out-of-date, a GitHub
release that isn't packaged yet, orphaned) or when the data is more than 13
hours old (the timer missed two runs). The tooltip lists every package with
its version, votes, downloads and alerts.

- 🖱️ **Left click:** opens the TUI in a floating, centered window, or brings the
  open one to your workspace.
- 🔄 **Right click:** fetches now (starts `aurwatch-fetch.service`).

It's a local-only plugin, installed by copying files into
`~/.config/omarchy/plugins/custom.aurwatch`, never through
`omarchy plugin add` or the marketplace. The widget polls
`aurwatch-toggle --summary` once a minute, which only reads `latest.json`;
it never touches the network. Window size: `AURWATCH_WIDTH`/`AURWATCH_HEIGHT`
(default 1180×680).

🗑️ To remove it from the bar: `omarchy plugin disable custom.aurwatch`.

## 🖥️ Terminal UI

Opens instantly from the last saved snapshot and refreshes in the background
when it's more than 6 hours old (or on `r`).

- 📋 **List:** every package with votes, popularity, downloads and alerts,
  plus a table of GitHub repo traffic.
- 🔍 **Details** (`enter`): package info and 30-day charts of downloads per day
  (`-bin` packages) and repo views and clones per day.
- ⌨️ **Keys:** `↑↓`/`jk` move, `enter` details, `esc` back, `r` refresh,
  `o` AUR page, `g` GitHub repo, `u` upstream URL, `?` help, `q` quit.

🎨 Colors come from the cybercore CYBERGRID theme; set `CYBERGRID_THEME` to use
another cybercore theme.

## 📊 Where the numbers come from

The AUR does not record downloads or page views, so aurwatch shows the closest
real signals and labels them for what they are.

| Number | Source | Caveat |
|---|---|---|
| ⭐ Votes, popularity, flagged, last updated | AUR RPC v5 | Popularity is the AUR's decayed vote score. |
| ⬇️ Downloads (`-bin` only) | GitHub release `download_count`, Linux assets only (no checksums, macOS or Windows) | Includes manual downloads. Source packages fetch tag archives, which GitHub doesn't count. |
| 👀 Views, clones | GitHub repo traffic | The GitHub repo, not the AUR page. Needs push access and covers only 14 days. |

🔑 GitHub requests use your existing `gh` login: aurwatch reads `GH_TOKEN` or
`GITHUB_TOKEN` if set, otherwise runs `gh auth token` once per fetch, then
calls the GitHub API directly.

## 📁 Files

- ⚙️ `~/.config/aurwatch/config.toml`: maintainer, extra packages, repo overrides.
- 🗂️ `~/.local/state/aurwatch/latest.json`: the last snapshot (read by the UI and widget).
- 🕰️ `~/.local/state/aurwatch/history.json`: history by UTC date. GitHub's
  per-day traffic is merged in on every fetch, so it grows past 14 days as long
  as `fetch` runs at least every two weeks. Download totals are recorded daily
  and the 7- and 30-day counts are differences between days.
- ⏱️ `contrib/systemd/`: the fetch service and timer (every 6 hours, catches up
  after the machine has been off). `install.sh` installs and enables them.
