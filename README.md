# Starlists

**Organize GitHub stars with official Star Lists.** Local-first CLI. A repo can sit in multiple lists. Classification can be done by you, Emacs, or an agent. GitHub only changes when you confirm.

GitHub’s own Lists are a public-preview grouping tool with no REST API, a ~32-list cap, and no notes or review workflow. Third-party apps either ignore Lists or lock classification inside their UI. Starlists treats **official Lists as the remote source of truth** and keeps planning on your machine.

## Install

```bash
cargo install --git https://github.com/chuxubank/starlists --locked
```

Or from a clone:

```bash
make install-local   # → ~/.local/bin/stars
```

The command name is `stars`.

## Quick start

```bash
stars init
stars --json doctor
stars snapshot                 # all starred repos + official Lists
stars lists
stars repos list --unlisted
```

Auth, in order: `STARS_GITHUB_TOKEN`, `GITHUB_TOKEN`, `GH_TOKEN`, `~/.config/stars/config.toml`, then `gh auth token`.

Writes to Lists need the `user` scope:

```bash
gh auth refresh -h github.com -s user
```

## How it works

1. **Snapshot** GitHub → SQLite (`~/.local/share/stars/stars.db`).
2. **Edit locally**: create/rename/delete lists, assign repos, import an agent proposal.
3. **Review** a plan (`stars apply --plan …` is a dry-run).
4. **`apply --confirm`** is the only command that writes GitHub.

`updateUserListsForItem` replaces a repo’s entire list membership. Apply always sends the **full set**. Deletes of Lists run **after** membership updates so repos are not stranded.

Draft lists can exceed 32; apply refuses if the live GitHub count would go over.

## Commands

```bash
stars --json doctor
stars snapshot
stars snapshot --stars-only          # refresh repo metadata only

stars lists
stars lists create "Web Mapping" --desc "…"
stars lists rename web-mapping "Web GIS"
stars lists delete web-mapping

stars repos list --unlisted
stars repos show owner/repo
stars assign owner/repo --add web-mapping --add emacs

stars export --for-agent --out corpus.json
stars propose import proposal.json
stars plan show
stars apply --plan PLAN_…            # dry-run
stars apply --plan PLAN_… --confirm

stars --json request --query 'query { viewer { login } }'
```

`--json` prints `{ "ok": true, "data": … }` or `{ "ok": false, "error": { "code", "message" } }` on stdout. Progress goes to stderr. Tokens are never printed. `--sexp` is for Emacs.

Exit codes: `0` ok, `1` generic, `2` auth, `3` not found, `4` conflict.

## Agents and Emacs

Proposal schema: [`schema/proposal.v1.json`](schema/proposal.v1.json). Example: [`examples/proposal.example.json`](examples/proposal.example.json).

```text
snapshot → export --for-agent → write proposal.json → propose import → plan show → apply --confirm
```

Do not let an agent pass `--confirm` or `request --write` unless you asked.

## Why not Astral / GithubStarsManager?

| | Starlists |
|---|---|
| Official Lists | Bidirectional, explicit apply |
| Classification | External (you / Emacs / any agent) |
| Data | SQLite on disk, inspectable |
| Writes | Dry-run default, full membership sets |

## License

MIT
