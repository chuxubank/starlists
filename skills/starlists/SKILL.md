---
name: starlists
description: >
  Operate the Starlists stars CLI for GitHub starred repos and official Star Lists.
  Use when snapshotting stars, planning or editing Lists, classifying with an
  external agent, applying list membership, or driving stars from Emacs.
  Triggers include GitHub stars, Star Lists, unlisted repos, and propose import.
---

# Starlists

Prefer the installed `stars` binary on PATH. Use `--json` when reading output. SQLite is local truth. GitHub Lists change only on `apply --confirm`.

## First commands

```bash
command -v stars
stars --json doctor
stars init
```

Auth: `GITHUB_TOKEN` / `GH_TOKEN` / `STARS_GITHUB_TOKEN` / `gh auth token`. List writes need `user` scope (`gh auth refresh -h github.com -s user`).

Database default: `~/.local/share/stars/stars.db`. Config: `~/.config/stars/config.toml`. Proposal schema: https://github.com/chuxubank/starlists/blob/main/schema/proposal.v1.json

## Safe read path

```bash
stars snapshot
stars --json lists
stars --json repos list --unlisted --limit 50
stars --json repos show owner/repo
stars export --for-agent --out /tmp/stars-corpus.json
```

`snapshot` is a network read. It updates the local DB. It does not write Lists.

## Classify

1. Export corpus (`export --for-agent`).
2. Write `proposal.json` matching `schema/proposal.v1.json`. A repo may have multiple list slugs. Prefer existing slugs. Create a list only for a real cluster (about 4+ repos). Do not split by programming language. Keep live GitHub lists around 20–28 (cap 32).
3. Import locally:

```bash
stars --json propose import proposal.json
stars --json plan show
```

Stop after import. Do not write GitHub from the classifier.

## Write path

Needs explicit user approval.

```bash
stars apply --plan PLAN_…
stars apply --plan PLAN_… --confirm
```

Without `--confirm` this is a dry-run. Pass `--confirm` or `request --write` only when the user asked to mutate GitHub.

Local-only edits:

```bash
stars lists create "Web Mapping" --desc "…"
stars assign owner/repo --add web-mapping --add emacs
```

`updateUserListsForItem` replaces a repo's entire list set. Apply always sends the full set. List deletes run after membership updates.

## Raw hatch

```bash
stars --json request --query 'query { viewer { login } }'
```

Mutations require `--write`.

## Emacs

`--sexp` prints `data` as an s-expression. Call the same binary.
