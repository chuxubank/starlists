use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::json;

use crate::cli::{Command, Format, ListsCommand, PlanCommand, ProposeCommand, ReposCommand};
use crate::config::{write_init, AppConfig};
use crate::db::{self, ListRow, RepoRow};
use crate::github::{looks_like_mutation, GitHub, GITHUB_LIST_CAP};
use crate::ids::new_plan_id;
use crate::output::{self, columns, kv_table, AppError};
use crate::proposal::{self, Plan, ProposalFile};

pub fn run(cfg: &AppConfig, format: Format, command: Command) -> Result<()> {
    match command {
        Command::Doctor => doctor(cfg, format),
        Command::Init => init(cfg, format),
        Command::Snapshot { stars_only } => snapshot(cfg, format, stars_only),
        Command::Lists { command } => lists(
            cfg,
            format,
            command.unwrap_or(ListsCommand::List { all: false }),
        ),
        Command::Repos { command } => repos(
            cfg,
            format,
            command.unwrap_or(ReposCommand::List {
                list: None,
                unlisted: false,
                limit: 50,
                query: None,
            }),
        ),
        Command::Assign {
            repo,
            add,
            remove,
            set,
        } => assign(cfg, format, &repo, add, remove, set),
        Command::Export { out, for_agent } => export(cfg, format, out, for_agent),
        Command::Propose(cmd) => propose(cfg, format, cmd),
        Command::Plan { command } => plan(cfg, format, command),
        Command::Apply { plan, confirm } => apply(cfg, format, &plan, confirm),
        Command::Request { query, vars, write } => request(cfg, format, &query, vars, write),
    }
}

fn open_db(cfg: &AppConfig) -> Result<Connection> {
    db::open(&cfg.paths.db_file)
}

fn doctor(cfg: &AppConfig, format: Format) -> Result<()> {
    let db_ok = cfg.paths.db_file.exists();
    let db_stats = if db_ok {
        open_db(cfg).ok().and_then(|c| db::stats(&c).ok())
    } else {
        None
    };
    let last_snapshot = open_db(cfg)
        .ok()
        .and_then(|c| db::get_meta(&c, "last_snapshot_at").ok().flatten());
    let gh_ok = if let Some(token) = &cfg.token {
        GitHub::new(token, &cfg.api_url)
            .and_then(|gh| gh.graphql("query { viewer { login } }", json!({})))
            .ok()
            .and_then(|v| v["viewer"]["login"].as_str().map(|s| s.to_string()))
    } else {
        None
    };

    #[derive(Serialize)]
    struct Doctor {
        version: &'static str,
        config_file: String,
        db_file: String,
        db_exists: bool,
        auth_source: &'static str,
        auth_present: bool,
        github_login: Option<String>,
        last_snapshot_at: Option<String>,
        stats: Option<serde_json::Value>,
        github_list_cap: usize,
    }

    let data = Doctor {
        version: env!("CARGO_PKG_VERSION"),
        config_file: cfg.paths.config_file.display().to_string(),
        db_file: cfg.paths.db_file.display().to_string(),
        db_exists: db_ok,
        auth_source: cfg.token_source.as_str(),
        auth_present: cfg.token.is_some(),
        github_login: gh_ok,
        last_snapshot_at: last_snapshot,
        stats: db_stats,
        github_list_cap: GITHUB_LIST_CAP,
    };
    output::success(format, &data, || {
        kv_table(&[
            ("version", data.version.to_string()),
            ("config", data.config_file.clone()),
            ("db", data.db_file.clone()),
            ("db_exists", data.db_exists.to_string()),
            ("auth", data.auth_source.to_string()),
            (
                "login",
                data.github_login.clone().unwrap_or_else(|| "-".into()),
            ),
            (
                "snapshot",
                data.last_snapshot_at.clone().unwrap_or_else(|| "-".into()),
            ),
        ])
    });
    Ok(())
}

fn init(cfg: &AppConfig, format: Format) -> Result<()> {
    let created = write_init(&cfg.paths)?;
    db::open(&cfg.paths.db_file)?;
    let data = json!({
        "config_file": cfg.paths.config_file,
        "db_file": cfg.paths.db_file,
        "created_config": created,
    });
    output::success(format, &data, || {
        format!(
            "config  {}\ndb      {}\n",
            cfg.paths.config_file.display(),
            cfg.paths.db_file.display()
        )
    });
    Ok(())
}

fn snapshot(cfg: &AppConfig, format: Format, stars_only: bool) -> Result<()> {
    let token = cfg.require_token()?;
    let gh = GitHub::new(token, &cfg.api_url)?;
    eprintln!("fetching starred repositories…");
    let (login, starred) = gh.fetch_starred()?;
    eprintln!("fetched {} starred repos as {login}", starred.len());

    let conn = open_db(cfg)?;
    let now = db::now_rfc3339();
    conn.execute("BEGIN", [])?;
    let persist_stars = (|| {
        let ids: Vec<i64> = starred.iter().map(|r| r.id).collect();
        for repo in &starred {
            db::upsert_repo(&conn, repo, &now)?;
        }
        let removed = db::delete_repos_not_in(&conn, &ids)?;
        db::set_meta(&conn, "last_snapshot_at", &now)?;
        db::set_meta(&conn, "github_login", &login)?;
        Ok::<_, anyhow::Error>(removed)
    })();
    let removed = match persist_stars {
        Ok(removed) => {
            conn.execute("COMMIT", [])?;
            removed
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(err);
        }
    };

    let lists = if stars_only {
        Vec::new()
    } else {
        eprintln!("fetching official lists…");
        match gh.fetch_lists() {
            Ok(lists) => {
                eprintln!("fetched {} lists", lists.len());
                lists
            }
            Err(err) => {
                return Err(err.context(
                    "starred repos were saved; list sync failed (re-run `stars snapshot`)",
                ));
            }
        }
    };

    conn.execute("BEGIN", [])?;
    let snap = (|| {
        if !stars_only {
            let remote_ids: Vec<String> = lists.iter().map(|l| l.github_id.clone()).collect();
            db::remove_remote_synced_lists_missing(&conn, &remote_ids)?;
            db::clear_synced_memberships(&conn)?;
            for list in &lists {
                for repo in &list.repos {
                    db::upsert_repo(&conn, repo, &now)?;
                }
                let list_id = db::upsert_synced_list(
                    &conn,
                    &list.github_id,
                    &list.name,
                    list.description.as_deref(),
                    list.is_private,
                    list.slug.as_deref(),
                    &now,
                )?;
                for repo in &list.repos {
                    db::add_membership(&conn, repo.id, list_id, "snapshot")?;
                }
            }
        }
        Ok::<_, anyhow::Error>(())
    })();
    match snap {
        Ok(()) => {
            conn.execute("COMMIT", [])?;
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK", []);
            return Err(err);
        }
    };

    let stats = db::stats(&conn)?;
    let data = json!({
        "login": login,
        "starred": starred.len(),
        "lists": lists.len(),
        "removed_repos": removed,
        "stats": stats,
        "snapshot_at": now,
    });
    output::success(format, &data, || {
        format!(
            "login     {login}\nstarred   {}\nlists     {}\nremoved   {}\nunlisted  {}\n",
            starred.len(),
            lists.len(),
            data["removed_repos"],
            stats["unlisted"]
        )
    });
    Ok(())
}

fn lists(cfg: &AppConfig, format: Format, cmd: ListsCommand) -> Result<()> {
    let conn = open_db(cfg)?;
    match cmd {
        ListsCommand::List { all } => {
            let rows = db::list_lists(&conn, all)?;
            output::success(format, &rows, || list_table(&rows));
        }
        ListsCommand::Resolve { name } => {
            let row = db::resolve_list(&conn, &name)?;
            output::success(format, &row, || list_detail(&row));
        }
        ListsCommand::Show { id, limit } => {
            let row = db::resolve_list(&conn, &id)?;
            let repos = db::query_repos(&conn, Some(&row), false, None, limit)?;
            let data = json!({ "list": row, "repos": repos });
            output::success(format, &data, || {
                let mut out = list_detail(&row);
                out.push_str(&repo_table(&repos));
                out
            });
        }
        ListsCommand::Create {
            name,
            desc,
            slug,
            public,
        } => {
            let row = db::create_list(&conn, &name, slug.as_deref(), desc.as_deref(), !public)?;
            output::success(format, &row, || list_detail(&row));
        }
        ListsCommand::Rename { id, name } => {
            let row = db::update_list(&conn, &id, Some(&name), None, None)?;
            output::success(format, &row, || list_detail(&row));
        }
        ListsCommand::Update { id, desc, public } => {
            let is_private = public.map(|p| !p);
            let row = db::update_list(&conn, &id, None, desc.as_deref().map(Some), is_private)?;
            output::success(format, &row, || list_detail(&row));
        }
        ListsCommand::Delete { id } => {
            let row = db::delete_list(&conn, &id)?;
            output::success(format, &row, || list_detail(&row));
        }
    }
    Ok(())
}

fn repos(cfg: &AppConfig, format: Format, cmd: ReposCommand) -> Result<()> {
    let conn = open_db(cfg)?;
    match cmd {
        ReposCommand::List {
            list,
            unlisted,
            limit,
            query,
        } => {
            let list_row = match list {
                Some(spec) => Some(db::resolve_list(&conn, &spec)?),
                None => None,
            };
            let rows =
                db::query_repos(&conn, list_row.as_ref(), unlisted, query.as_deref(), limit)?;
            output::success(format, &rows, || repo_table(&rows));
        }
        ReposCommand::Show { id } => {
            let row = db::resolve_repo(&conn, &id)?;
            output::success(format, &row, || repo_detail(&row));
        }
    }
    Ok(())
}

fn assign(
    cfg: &AppConfig,
    format: Format,
    repo: &str,
    add: Vec<String>,
    remove: Vec<String>,
    set: Vec<String>,
) -> Result<()> {
    let conn = open_db(cfg)?;
    let repo = db::resolve_repo(&conn, repo)?;
    let next = if !set.is_empty() {
        set
    } else {
        let mut slugs = db::membership_slugs(&conn, repo.id)?;
        for spec in add {
            slugs.push(db::resolve_list(&conn, &spec)?.slug);
        }
        let remove: Vec<String> = remove
            .iter()
            .map(|s| db::resolve_list(&conn, s).map(|l| l.slug))
            .collect::<Result<Vec<_>>>()?;
        slugs.retain(|s| !remove.contains(s));
        slugs.sort();
        slugs.dedup();
        slugs
    };
    let ids = next
        .iter()
        .map(|s| db::resolve_list(&conn, s).map(|l| l.id))
        .collect::<Result<Vec<_>>>()?;
    db::set_membership(&conn, repo.id, &ids, "local")?;
    let repo = db::resolve_repo(&conn, &repo.name_with_owner)?;
    output::success(format, &repo, || repo_detail(&repo));
    Ok(())
}

fn export(cfg: &AppConfig, format: Format, out: Option<PathBuf>, for_agent: bool) -> Result<()> {
    let conn = open_db(cfg)?;
    let lists = db::list_lists(&conn, false)?;
    let repos = db::all_repos(&conn)?;
    let unlisted: Vec<_> = repos
        .iter()
        .filter(|r| r.lists.is_empty())
        .cloned()
        .collect();
    let corpus = if for_agent {
        json!({
            "version": 1,
            "instructions": [
                "Classify starred repos into the user's Star Lists.",
                "A repo may belong to multiple lists (typically 1-3).",
                "Prefer existing list slugs. Create a list only for a real cluster of 4+ repos.",
                "Do not classify by programming language; use purpose or domain.",
                "Keep surviving GitHub lists around 20-28 so a few slots remain (cap 32).",
                "Write a proposal JSON matching schema/proposal.v1.json. Do not call GitHub."
            ],
            "list_cap": GITHUB_LIST_CAP,
            "lists": lists,
            "repos": repos.iter().map(agent_repo).collect::<Vec<_>>(),
            "unlisted": unlisted.iter().map(|r| r.name_with_owner.clone()).collect::<Vec<_>>(),
            "example_proposal": proposal::example_proposal(),
        })
    } else {
        json!({ "lists": lists, "repos": repos })
    };

    if let Some(path) = out {
        fs::write(&path, serde_json::to_string_pretty(&corpus)?)?;
        let data = json!({ "path": path, "repos": repos.len(), "lists": lists.len() });
        output::success(format, &data, || format!("wrote {}\n", path.display()));
    } else if matches!(format, Format::Table) {
        println!("{}", serde_json::to_string_pretty(&corpus)?);
    } else {
        output::success(format, &corpus, || String::new());
    }
    Ok(())
}

fn agent_repo(repo: &RepoRow) -> serde_json::Value {
    json!({
        "repo": repo.name_with_owner,
        "description": repo.description,
        "language": repo.primary_language,
        "topics": repo.topics,
        "stars": repo.stars,
        "lists": repo.lists,
        "url": repo.url,
    })
}

fn propose(cfg: &AppConfig, format: Format, cmd: ProposeCommand) -> Result<()> {
    let conn = open_db(cfg)?;
    match cmd {
        ProposeCommand::Import { path, replace } => {
            let raw = fs::read_to_string(&path)?;
            let parsed = proposal::parse_proposal(&raw)?;
            let id = parsed
                .id
                .clone()
                .unwrap_or_else(|| new_plan_id(&db::now_rfc3339()));
            if db::get_proposal(&conn, &id)?.is_some() {
                if replace {
                    db::replace_proposal(
                        &conn,
                        &id,
                        parsed.notes.as_deref(),
                        Some(&path.display().to_string()),
                        &raw,
                    )?;
                } else {
                    return Err(
                        AppError::new("conflict", format!("proposal {id} already exists"))
                            .hint("pass --replace")
                            .into(),
                    );
                }
            } else {
                db::insert_proposal(
                    &conn,
                    &id,
                    parsed.notes.as_deref(),
                    Some(&path.display().to_string()),
                    &raw,
                )?;
            }
            let plan = proposal::compile_plan(&conn, &id, &parsed)?;
            let applied = proposal::apply_proposal_locally(&conn, &parsed)?;
            let data = json!({ "id": id, "local_changes": applied, "plan": plan });
            output::success(format, &data, || {
                format!(
                    "imported {id}\nlocal    {} changes\nops      {}\nmembers  {}\n",
                    applied.len(),
                    plan.list_ops.len(),
                    plan.memberships.len()
                )
            });
        }
        ProposeCommand::Show { id } => {
            let row = load_proposal(&conn, id.as_deref())?;
            let parsed: ProposalFile = proposal::parse_proposal(&row.raw_json)?;
            output::success(format, &parsed, || {
                serde_json::to_string_pretty(&parsed).unwrap_or_default() + "\n"
            });
        }
    }
    Ok(())
}

fn plan(cfg: &AppConfig, format: Format, cmd: PlanCommand) -> Result<()> {
    let conn = open_db(cfg)?;
    match cmd {
        PlanCommand::Show { id } => {
            let (row, plan) = load_plan(&conn, id.as_deref())?;
            output::success(format, &plan, || plan_table(&row.id, &plan));
        }
    }
    Ok(())
}

fn apply(cfg: &AppConfig, format: Format, plan_id: &str, confirm: bool) -> Result<()> {
    let conn = open_db(cfg)?;
    let (row, plan) = load_plan(&conn, Some(plan_id))?;
    if plan.list_cap.after_apply > plan.list_cap.max {
        return Err(AppError::new(
            "conflict",
            format!(
                "apply would leave {} GitHub lists (cap {})",
                plan.list_cap.after_apply, plan.list_cap.max
            ),
        )
        .into());
    }
    if !confirm {
        let data = json!({ "dry_run": true, "plan": plan });
        output::success(format, &data, || {
            let mut out = String::from("dry-run; pass --confirm to write GitHub\n");
            out.push_str(&plan_table(&row.id, &plan));
            out
        });
        return Ok(());
    }

    let token = cfg.require_token()?;
    let gh = GitHub::new(token, &cfg.api_url)?;
    let mut created: std::collections::HashMap<String, String> = Default::default();
    let mut results = Vec::new();

    let mut deletes = Vec::new();
    for op in &plan.list_ops {
        match op {
            proposal::ListOp::Create {
                slug,
                name,
                description,
                is_private,
            } => {
                let created_list = gh.create_list(name, description.as_deref(), *is_private)?;
                created.insert(slug.clone(), created_list.github_id.clone());
                let now = db::now_rfc3339();
                conn.execute(
                    "UPDATE list SET github_id=?1, status='synced', updated_at=?2 WHERE slug=?3",
                    rusqlite::params![created_list.github_id, now, slug],
                )?;
                results.push(json!({ "op": "createUserList", "slug": slug, "github_id": created_list.github_id }));
            }
            proposal::ListOp::Update {
                slug,
                github_id,
                name,
                description,
                is_private,
            } => {
                let gid = github_id
                    .clone()
                    .or_else(|| created.get(slug).cloned())
                    .ok_or_else(|| {
                        AppError::new(
                            "conflict",
                            format!("cannot update {slug} before it exists on GitHub"),
                        )
                    })?;
                gh.update_list(&gid, name.as_deref(), description.as_deref(), *is_private)?;
                let now = db::now_rfc3339();
                conn.execute(
                    "UPDATE list SET status='synced', updated_at=?1 WHERE slug=?2",
                    rusqlite::params![now, slug],
                )?;
                results.push(json!({ "op": "updateUserList", "slug": slug, "github_id": gid }));
            }
            proposal::ListOp::Delete { slug, github_id } => {
                deletes.push((slug.clone(), github_id.clone()));
            }
        }
    }

    for membership in &plan.memberships {
        let mut ids = Vec::new();
        for (slug, pending) in membership
            .lists
            .iter()
            .zip(membership.list_github_ids.iter())
        {
            if let Some(rest) = pending.strip_prefix("pending:") {
                let gid = created
                    .get(rest)
                    .or_else(|| created.get(slug))
                    .ok_or_else(|| {
                        AppError::new("conflict", format!("missing GitHub id for list {slug}"))
                    })?;
                ids.push(gid.clone());
            } else {
                ids.push(pending.clone());
            }
        }
        let names = gh.set_item_lists(&membership.repo_node_id, &ids)?;
        results.push(json!({
            "op": "updateUserListsForItem",
            "repo": membership.repo,
            "lists": names,
        }));
    }

    for (slug, github_id) in deletes {
        if let Some(gid) = github_id {
            gh.delete_list(&gid)?;
        }
        conn.execute("DELETE FROM list WHERE slug = ?1", rusqlite::params![slug])?;
        results.push(json!({ "op": "deleteUserList", "slug": slug }));
    }

    db::mark_proposal_applied(&conn, &row.id)?;
    let data = json!({ "dry_run": false, "plan_id": row.id, "results": results });
    output::success(format, &data, || {
        format!("applied {}\n{} GitHub mutations\n", row.id, results.len())
    });
    Ok(())
}

fn request(
    cfg: &AppConfig,
    format: Format,
    query: &str,
    vars: Option<String>,
    write: bool,
) -> Result<()> {
    if looks_like_mutation(query) && !write {
        return Err(AppError::new("conflict", "GraphQL mutation requires --write").into());
    }
    let token = cfg.require_token()?;
    let gh = GitHub::new(token, &cfg.api_url)?;
    let variables = match vars {
        Some(raw) => serde_json::from_str(&raw)
            .map_err(|e| AppError::new("invalid", format!("vars JSON: {e}")))?,
        None => json!({}),
    };
    let data = gh.graphql(query, variables)?;
    output::success(format, &data, || {
        serde_json::to_string_pretty(&data).unwrap_or_default() + "\n"
    });
    Ok(())
}

fn load_proposal(conn: &Connection, id: Option<&str>) -> Result<db::ProposalRow> {
    match id {
        Some(id) => db::get_proposal(conn, id)?
            .ok_or_else(|| AppError::new("not_found", format!("proposal not found: {id}")).into()),
        None => db::latest_proposal(conn)?
            .ok_or_else(|| AppError::new("not_found", "no proposal imported yet").into()),
    }
}

fn load_plan(conn: &Connection, id: Option<&str>) -> Result<(db::ProposalRow, Plan)> {
    let row = load_proposal(conn, id)?;
    let parsed = proposal::parse_proposal(&row.raw_json)?;
    let plan = proposal::compile_plan(conn, &row.id, &parsed)?;
    Ok((row, plan))
}

fn list_table(rows: &[ListRow]) -> String {
    columns(
        &["SLUG", "NAME", "STATUS", "N", "GITHUB"],
        &rows
            .iter()
            .map(|r| {
                vec![
                    r.slug.clone(),
                    r.name.clone(),
                    r.status.clone(),
                    r.repo_count.to_string(),
                    r.github_id.clone().unwrap_or_else(|| "-".into()),
                ]
            })
            .collect::<Vec<_>>(),
    )
}

fn list_detail(row: &ListRow) -> String {
    kv_table(&[
        ("id", row.id.to_string()),
        ("slug", row.slug.clone()),
        ("name", row.name.clone()),
        ("status", row.status.clone()),
        ("private", row.is_private.to_string()),
        ("repos", row.repo_count.to_string()),
        (
            "github",
            row.github_id.clone().unwrap_or_else(|| "-".into()),
        ),
        (
            "desc",
            row.description.clone().unwrap_or_else(|| "-".into()),
        ),
    ])
}

fn repo_table(rows: &[RepoRow]) -> String {
    columns(
        &["REPO", "LANG", "STARS", "LISTS"],
        &rows
            .iter()
            .map(|r| {
                vec![
                    r.name_with_owner.clone(),
                    r.primary_language.clone().unwrap_or_else(|| "-".into()),
                    r.stars.map(|n| n.to_string()).unwrap_or_else(|| "-".into()),
                    if r.lists.is_empty() {
                        "-".into()
                    } else {
                        r.lists.join(",")
                    },
                ]
            })
            .collect::<Vec<_>>(),
    )
}

fn repo_detail(row: &RepoRow) -> String {
    kv_table(&[
        ("repo", row.name_with_owner.clone()),
        ("url", row.url.clone()),
        (
            "lang",
            row.primary_language.clone().unwrap_or_else(|| "-".into()),
        ),
        (
            "stars",
            row.stars
                .map(|n| n.to_string())
                .unwrap_or_else(|| "-".into()),
        ),
        ("lists", row.lists.join(", ")),
        (
            "desc",
            row.description.clone().unwrap_or_else(|| "-".into()),
        ),
        ("topics", row.topics.join(", ")),
    ])
}

fn plan_table(id: &str, plan: &Plan) -> String {
    let mut out = format!(
        "plan      {id}\nremote    {} -> {} / {}\nops       {}\nmembers   {}\n",
        plan.list_cap.current_remote,
        plan.list_cap.after_apply,
        plan.list_cap.max,
        plan.list_ops.len(),
        plan.memberships.len()
    );
    for warning in &plan.warnings {
        out.push_str(&format!("warning   {warning}\n"));
    }
    out
}
