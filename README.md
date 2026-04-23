# mjira

A Rust CLI for interacting with Jira (Cloud and Server/Data Center) from the terminal.

## Installation

```bash
just install   # builds release binary, copies to ~/bin/mjira, and installs zsh completions to ~/.zfunc/_mjira
```

Or build manually:

```bash
cargo build --release
cp target/release/jira ~/bin/mjira
```

## Shell completions

Add to your `.zshrc`:

```zsh
eval "$(mjira completions zsh)"
```

For other shells:

```bash
# bash — add to ~/.bashrc
eval "$(mjira completions bash)"

# fish — add to ~/.config/fish/config.fish
mjira completions fish | source
```

The zsh completion includes dynamic issue key completion with summaries shown alongside each candidate. Results are fetched from Jira (issues updated in the last 90 days, up to 200) and cached for 5 minutes.

Cache location:

| Platform | Path |
|---|---|
| macOS | `~/Library/Caches/makrel/<instance>/issues` |
| Linux | `~/.cache/makrel/<instance>/issues` (or `$XDG_CACHE_HOME/makrel/<instance>/issues`) |

## Configuration

The config file location depends on your platform:

| Platform | Path |
|---|---|
| macOS | `~/Library/Application Support/mjira/config.toml` |
| Linux | `~/.config/mjira/config.toml` (or `$XDG_CONFIG_HOME/mjira/config.toml`) |
| Windows | `%APPDATA%\mjira\config.toml` |

Copy `config.example.toml` as a starting point.

```bash
mjira instance path   # print the config file location
```

### Adding an instance

Interactive:

```bash
mjira instance add work
```

Or non-interactive:

```bash
# Jira Cloud (API token)
mjira instance add cloud \
  --url https://mycompany.atlassian.net \
  --username user@mycompany.com \
  --api-key ATATT3xFfGF0...

# Jira Server (password)
mjira instance add server \
  --url https://jira.internal.example.com \
  --username john.doe \
  --password s3cr3t
```

For Jira Server 8.14+ / Data Center with 2FA, use a Personal Access Token — add `pat = "..."` directly in the config file.

### Managing instances

```bash
mjira instance list               # list all configured instances
mjira instance set-default cloud  # change the default
mjira instance remove old         # remove an instance
```

### Selecting an instance at runtime

```bash
mjira --instance server issue list
JIRA_INSTANCE=server mjira issue list   # via environment variable
```

## Issues

### List issues

By default shows issues assigned to the current user (or `default_assignee` in config).

```bash
mjira issue list
mjira issue list --project PROJ
mjira issue list --status "In Progress"
mjira issue list --type Bug
mjira issue list --assignee john.doe
mjira issue list --any-assignee          # remove the assignee filter entirely
mjira issue list --limit 50
mjira issue list --jql "priority = High" # append extra JQL
```

Customize columns:

```bash
mjira issue list --columns key,type,status,priority,summary
mjira issue list --list-columns   # show all available column names
```

### View an issue

```bash
mjira issue get PROJ-123
mjira issue get PROJ-123 --images   # also render image attachments via kitty icat
```

Displays summary, metadata, description, comments, and assignee history. The `--images` flag downloads image attachments and renders them inline using [kitty's icat kitten](https://sw.kovidgoyal.net/kitty/kittens/icat/).

### Create an issue

```bash
mjira issue create --project PROJ --summary "Fix login bug"
mjira issue create --project PROJ --summary "Add dark mode" --type Story
mjira issue create --project PROJ --summary "Urgent crash" --type Bug --priority High
mjira issue create --project PROJ --summary "Task" --description "Details here" --assignee john.doe
```

### Transition an issue

```bash
mjira issue transition PROJ-123                  # list available transitions
mjira issue transition PROJ-123 "In Progress"    # move to a status (case-insensitive)
mjira issue transition PROJ-123 done
mjira issue transition PROJ-123 done --assign          # assign to default_assignee after transition
mjira issue transition PROJ-123 done --assign john.doe # assign to a specific user after transition
mjira issue transition PROJ-123 done --unassign        # unassign after transition
```

### Assign an issue

```bash
mjira issue assign PROJ-123 john.doe   # assign
mjira issue assign PROJ-123 -          # unassign
```

### Comment on an issue

```bash
mjira issue comment PROJ-123 "Looks good to me."
```

### List valid field values

```bash
mjira issue values status
mjira issue values priority
mjira issue values "Fix Version/s" --project PROJ
```

## Git integration

Find commits that mention an issue key across your configured repositories.

```bash
mjira issue commits PROJ-123
mjira issue commits PROJ-123 --repo /path/to/extra/repo
mjira issue commits PROJ-123 --verbose   # also show repos with no results
```

Show the full diff for those commits:

```bash
mjira issue diff PROJ-123
mjira issue diff PROJ-123 --commit abc1234   # diff for a specific commit only
```

Repositories are resolved from the instance config. If the issue has components, `component_repos` mappings take priority over the `repos` fallback list. See `config.example.toml` for the format.

## Search

Quick text search across summary, description, and assignee:

```bash
mjira search -t "login bug"
mjira search -t "alice" --limit 50
mjira search -t "payment" --columns key,type,status,summary
```

Or run an ad-hoc JQL query directly:

```bash
mjira search 'project = PROJ AND status = "In Progress" ORDER BY updated DESC'
mjira search 'assignee = currentUser()' --limit 50
mjira search 'priority = High' --columns key,type,status,summary
mjira search --list-columns   # show available column names
```

> **Note:** The `--text` search uses `assignee = "TERM"` for the assignee clause, which requires an exact username or account ID on Jira Cloud. On Server/Data Center it matches by username.

## Saved queries

Define named queries in the config file (see path above):

```toml
[queries.my-work]
jql = "assignee = currentUser() AND status != Done ORDER BY updated DESC"

[queries.review]
jql     = "status = \"In Review\" ORDER BY updated DESC"
columns = "key,type,status,assignee,summary"
limit   = 20
```

Run them:

```bash
mjira query             # list all saved queries
mjira query my-work
mjira query review --limit 10 --columns key,status,summary   # override config defaults
```

## Projects

```bash
mjira project list
mjira project list --search platform
```

## Boards

Requires the Agile/Software plugin (standard on Jira Cloud, optional on Server/Data Center).

### List boards

```bash
mjira board list                        # all accessible boards (auto-paginated)
mjira board list --project PROJ         # filter by project key or ID
mjira board list --name "My Team"       # filter by board name (substring)
mjira board list --limit 20             # return at most 20 boards
```

### List issues on a board

```bash
mjira board issues 42
mjira board issues 42 --limit 50
mjira board issues 42 --jql 'status = "In Progress"'
mjira board issues 42 --columns key,type,status,priority,summary
mjira board issues 42 --quick-filter 10          # apply a quick filter by ID
mjira board issues 42 --quick-filter 10 --jql 'priority = High'  # combine with extra JQL
```

### Quick filters

```bash
mjira board quick-filters 42   # list all quick filters for board 42 (auto-paginated)
```

Each quick filter has an ID, name, and its underlying JQL clause. Use the ID with `board issues --quick-filter`.

Older Jira Server instances that do not expose the quick-filter endpoint on the Agile REST API are handled automatically via a fallback to the legacy GreenHopper API.


## Global flags

| Flag | Description |
|---|---|
| `--instance <alias>` | Use a specific instance (or set `JIRA_INSTANCE`) |
| `--verbose` | Print each HTTP request to stderr |
