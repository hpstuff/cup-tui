# cup-tui

A fullscreen terminal UI for ClickUp, built on top of the [`cup`](https://www.npmjs.com/package/@clickup/cup) CLI.
Browse spaces → folders → lists → tasks, read and edit tasks, comment, change status, all from the keyboard.

```
 cup-tui  NRG › Product Finder App › Development › 2h745c7                                Rumen Rusanov
┏ Workspace ━━━━━━━━━━━━━┓╭ Development 89 ────────────────────────────╮╭ Task 2h745c7 ─────────────────╮
┃MINE                    ┃│ID        Status      Pri    Name      Due   ││ Technical Walk Through …      │
┃◉ My tasks              ┃│2h745c7   in progress high   Technica…       ││ Status  in progress           │
┃✉ Inbox                 ┃│…                                             ││ Assignees …                   │
┃⚠ Overdue               ┃│                                              ││                               │
┃SPACES                  ┃│                                              ││ Description                   │
┃▾ NRG                   ┃│                                              ││ …                             │
┃  ▾ Product Finder App  ┃│                                              ││ Comments 3                    │
┃    ≡ Development 75    ┃│                                              ││ …                             │
```

## Requirements

- `cup` installed and configured (`cup init`). Writes, task details, comments, search and the workspace tree go through `cup … --json`.
  Task lists and "My tasks" are fetched directly from the ClickUp API with the token cup stored (same `~/.config/cup/config.json`, same profiles, same `CU_API_TOKEN` / `CU_TEAM_ID` overrides), because cup's list output has no assignees and no paging. If that config cannot be read, lists fall back to cup.
- Rust toolchain (`cargo`) to build.
- Optional: `$VISUAL` / `$EDITOR` for comments and descriptions (falls back to `vi`).

## Install

With Homebrew (macOS and Linux). This repository is also the tap, so it is tapped by URL once:

```bash
brew tap hpstuff/cup-tui https://github.com/hpstuff/cup-tui
brew install cup-tui
```

Homebrew asks you to trust third-party taps the first time; answer yes, or run
`brew trust --tap https://github.com/hpstuff/cup-tui` beforehand. `brew install --HEAD cup-tui` builds the
latest `main` instead of the last release. The formula builds from source with Homebrew's `rust`, so the first
install takes a minute or two. Later versions arrive with `brew update && brew upgrade cup-tui`.

From a checkout:

```bash
cargo install --path .
```

Or just build and run:

```bash
cargo build --release
./target/release/cup-tui
```

### Releasing (maintainers)

`scripts/release.sh X.Y.Z` bumps the version, tags, pushes, creates the GitHub release and updates
`Formula/cup-tui.rb` with the tarball checksum. Because the repo is the tap, that push is the release.

## Layout

Three panes: **Workspace** tree on the left, **tasks** table in the middle, **task detail** on the right (opens on Enter).
Tasks are grouped by the parent task's status (open → in-progress-style → done → closed, then the list's own order); subtasks follow their parent, indented with `↳`, showing their own status. A subtask whose parent is not in the view is listed on its own with a dim `↑ parent` hint. `S` switches to flat sorts.
Big lists arrive 100 tasks at a time: the first page shows up right away and the rest stream in behind it (`…` after the count while that is still happening).
On terminals narrower than 110 columns the detail replaces the table instead of splitting it.
The top bar is a breadcrumb; the bottom bar shows the keys for the focused pane and the result of the last action.

## Keys

Press `?` inside the app for the same list.

| Keys | Action |
| --- | --- |
| `Tab` / `Shift-Tab`, `1` `2` `3` | cycle / jump between panes |
| `j` `k` `↑` `↓`, `PgUp` `PgDn`, `Home` `End`, `Ctrl-d` `Ctrl-u` | move |
| `Enter` | sidebar: open list or fold · tasks: open detail |
| `h` `l` `←` `→` | sidebar: fold / unfold · tasks: collapse / expand subtasks, then sidebar / detail |
| `Space` | show / hide the subtasks of the parent under the cursor (parents start collapsed) |
| `-` / `=` | collapse / expand all parent tasks |
| `Esc` | clear text filter → clear filters → close detail |
| `[` `]` | previous / next task while reading a detail |
| `b` | toggle the sidebar |
| `/` | live fuzzy text filter in the focused pane |
| `f` | filter by status, priority, assignee (multi-select: `Tab` toggles, `Enter` applies) |
| `g` | search tasks in the current space (or my tasks when no space is selected) |
| `Ctrl-g` | search the whole workspace (slow, cup walks every list) |
| `x` | include closed tasks (lists, searches, My tasks) |
| `S` | sort: status, priority, due, name, ClickUp order |
| `r` / `R` | refresh the focused pane / refresh everything |
| `s` `p` `a` `m` | status · priority · assignee picker · assign or unassign me |
| `c` `E` | comment · edit description (opens `$EDITOR`) |
| `e` `d` `t` | rename · due date · tags (`a, b, -old`) |
| `n` `N` | new task in this list · new subtask of the selected task |
| `o` `y` `Y` | open in browser · copy URL · copy ID |
| detail: `h` `j` `k` `l` | move the cursor (`w` `b` words, `0` `$` line, `gg` `G` top/bottom, `Ctrl-d` `Ctrl-u` pages); the view scrolls with it |
| detail: `v` / `V` | select by character / by line from the cursor; `y` copies, `o` swaps ends, `Esc` cancels. Click places the cursor. |
| `q` / `Ctrl-c` | quit |

Mouse: wheel scrolls the pane under the cursor, click selects, clicking a selected task opens it.

## How it works

- Every screen is backed by an async request (a `cup` subprocess or one ClickUp API page), so the UI never blocks; a spinner in the status bar shows requests in flight.
- Results are cached per list / task and refreshed on `r`, after any write, or when a task detail is older than 45 seconds.
- Status and assignee filters are sent to the API (a new page 0 per filter set); the priority filter and the text filter are applied locally.
- Writes use the matching cup commands: `cup update`, `cup assign`, `cup comment`, `cup tag`, `cup create`. Long text (comments, descriptions) goes through a temp file and `--message-file` / `--description-file`, so quoting is never an issue.
- Statuses offered in the status picker come from the space definition plus whatever statuses the current list is actually using; cup fuzzy-matches the name on update.

## Not covered (yet)

Deleting or archiving tasks, time tracking, docs, goals, checklists editing, attachments upload, custom-field editing. All of these exist in `cup` and are straightforward to add as actions in `src/app.rs`.
