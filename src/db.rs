use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{slugify, unique_slug};
use crate::output::AppError;

pub const SCHEMA_VERSION: i64 = 1;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS repo (
    id INTEGER PRIMARY KEY,
    node_id TEXT NOT NULL UNIQUE,
    name_with_owner TEXT NOT NULL UNIQUE,
    url TEXT NOT NULL,
    description TEXT,
    homepage_url TEXT,
    primary_language TEXT,
    license TEXT,
    stars INTEGER,
    forks INTEGER,
    is_private INTEGER NOT NULL DEFAULT 0,
    is_archived INTEGER NOT NULL DEFAULT 0,
    is_fork INTEGER NOT NULL DEFAULT 0,
    pushed_at TEXT,
    updated_at TEXT,
    starred_at TEXT,
    topics_json TEXT NOT NULL DEFAULT '[]',
    snapshot_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS list (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    slug TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    is_private INTEGER NOT NULL DEFAULT 1,
    github_id TEXT UNIQUE,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS membership (
    repo_id INTEGER NOT NULL REFERENCES repo(id) ON DELETE CASCADE,
    list_id INTEGER NOT NULL REFERENCES list(id) ON DELETE CASCADE,
    source TEXT NOT NULL DEFAULT 'local',
    PRIMARY KEY (repo_id, list_id)
);

CREATE TABLE IF NOT EXISTS proposal (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL,
    notes TEXT,
    status TEXT NOT NULL,
    source_path TEXT,
    raw_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_list_status ON list(status);
CREATE INDEX IF NOT EXISTS idx_list_name ON list(name);
CREATE INDEX IF NOT EXISTS idx_membership_list ON membership(list_id);
CREATE INDEX IF NOT EXISTS idx_repo_name ON repo(name_with_owner);
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRow {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub is_private: bool,
    pub github_id: Option<String>,
    pub status: String,
    pub repo_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoRow {
    pub id: i64,
    pub node_id: String,
    pub name_with_owner: String,
    pub url: String,
    pub description: Option<String>,
    pub homepage_url: Option<String>,
    pub primary_language: Option<String>,
    pub license: Option<String>,
    pub stars: Option<i64>,
    pub forks: Option<i64>,
    pub is_private: bool,
    pub is_archived: bool,
    pub is_fork: bool,
    pub pushed_at: Option<String>,
    pub updated_at: Option<String>,
    pub starred_at: Option<String>,
    pub topics: Vec<String>,
    pub lists: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalRow {
    pub id: String,
    pub created_at: String,
    pub notes: Option<String>,
    pub status: String,
    pub source_path: Option<String>,
    pub raw_json: String,
}

pub fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn =
        Connection::open(path).with_context(|| format!("open database {}", path.display()))?;
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
    conn.execute_batch(SCHEMA)?;
    let version: i64 = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if version == 0 {
        conn.execute(
            "INSERT OR REPLACE INTO meta(key, value) VALUES ('schema_version', ?1)",
            params![SCHEMA_VERSION.to_string()],
        )?;
    } else if version != SCHEMA_VERSION {
        anyhow::bail!("unsupported schema version {version}, expected {SCHEMA_VERSION}");
    }
    Ok(conn)
}

pub fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

pub fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

pub fn get_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| {
            r.get(0)
        })
        .optional()?)
}

pub fn upsert_repo(conn: &Connection, repo: &RepoRow, snapshot_at: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO repo (
            id, node_id, name_with_owner, url, description, homepage_url,
            primary_language, license, stars, forks, is_private, is_archived,
            is_fork, pushed_at, updated_at, starred_at, topics_json, snapshot_at
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)
        ON CONFLICT(id) DO UPDATE SET
            node_id=excluded.node_id,
            name_with_owner=excluded.name_with_owner,
            url=CASE WHEN excluded.url != '' THEN excluded.url ELSE repo.url END,
            description=COALESCE(excluded.description, repo.description),
            homepage_url=COALESCE(excluded.homepage_url, repo.homepage_url),
            primary_language=COALESCE(excluded.primary_language, repo.primary_language),
            license=COALESCE(excluded.license, repo.license),
            stars=COALESCE(excluded.stars, repo.stars),
            forks=COALESCE(excluded.forks, repo.forks),
            is_private=excluded.is_private,
            is_archived=excluded.is_archived,
            is_fork=excluded.is_fork,
            pushed_at=COALESCE(excluded.pushed_at, repo.pushed_at),
            updated_at=COALESCE(excluded.updated_at, repo.updated_at),
            starred_at=COALESCE(excluded.starred_at, repo.starred_at),
            topics_json=CASE
                WHEN excluded.topics_json IS NOT NULL AND excluded.topics_json != '[]'
                THEN excluded.topics_json ELSE repo.topics_json END,
            snapshot_at=excluded.snapshot_at
        ",
        params![
            repo.id,
            repo.node_id,
            repo.name_with_owner,
            repo.url,
            repo.description,
            repo.homepage_url,
            repo.primary_language,
            repo.license,
            repo.stars,
            repo.forks,
            repo.is_private as i64,
            repo.is_archived as i64,
            repo.is_fork as i64,
            repo.pushed_at,
            repo.updated_at,
            repo.starred_at,
            serde_json::to_string(&repo.topics)?,
            snapshot_at,
        ],
    )?;
    Ok(())
}

pub fn delete_repos_not_in(conn: &Connection, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(conn.execute("DELETE FROM repo", [])?);
    }
    let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("DELETE FROM repo WHERE id NOT IN ({placeholders})");
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::types::ToSql> = ids
        .iter()
        .map(|id| id as &dyn rusqlite::types::ToSql)
        .collect();
    Ok(stmt.execute(rusqlite::params_from_iter(params))?)
}

pub fn all_slugs(conn: &Connection) -> Result<Vec<String>> {
    let mut stmt = conn.prepare("SELECT slug FROM list")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn upsert_synced_list(
    conn: &Connection,
    github_id: &str,
    name: &str,
    description: Option<&str>,
    is_private: bool,
    github_slug: Option<&str>,
    now: &str,
) -> Result<i64> {
    if let Some(existing) = find_list_by_github_id(conn, github_id)? {
        if existing.status == "synced" {
            conn.execute(
                "UPDATE list SET name=?1, description=?2, is_private=?3, updated_at=?4 WHERE id=?5",
                params![name, description, is_private as i64, now, existing.id],
            )?;
            return Ok(existing.id);
        }
        conn.execute(
            "UPDATE list SET github_id=?1, updated_at=?2 WHERE id=?3",
            params![github_id, now, existing.id],
        )?;
        return Ok(existing.id);
    }

    if let Some(existing) = find_list_by_name_ci(conn, name)?.filter(|l| l.github_id.is_none()) {
        conn.execute(
            "UPDATE list SET github_id=?1, name=?2, description=?3, is_private=?4, status='synced', updated_at=?5 WHERE id=?6",
            params![github_id, name, description, is_private as i64, now, existing.id],
        )?;
        return Ok(existing.id);
    }

    let taken = all_slugs(conn)?;
    let base = github_slug
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| slugify(name));
    let slug = unique_slug(&base, &taken);
    conn.execute(
        "INSERT INTO list (slug, name, description, is_private, github_id, status, created_at, updated_at)
         VALUES (?1,?2,?3,?4,?5,'synced',?6,?6)",
        params![slug, name, description, is_private as i64, github_id, now],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn clear_synced_memberships(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM membership WHERE list_id IN (SELECT id FROM list WHERE status = 'synced')",
        [],
    )?;
    Ok(())
}

pub fn add_membership(conn: &Connection, repo_id: i64, list_id: i64, source: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO membership (repo_id, list_id, source) VALUES (?1,?2,?3)",
        params![repo_id, list_id, source],
    )?;
    Ok(())
}

pub fn remove_remote_synced_lists_missing(
    conn: &Connection,
    present_github_ids: &[String],
) -> Result<usize> {
    if present_github_ids.is_empty() {
        return Ok(conn.execute(
            "DELETE FROM list WHERE status = 'synced' AND github_id IS NOT NULL",
            [],
        )?);
    }
    let placeholders = present_github_ids
        .iter()
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "DELETE FROM list WHERE status = 'synced' AND github_id IS NOT NULL AND github_id NOT IN ({placeholders})"
    );
    let mut stmt = conn.prepare(&sql)?;
    Ok(stmt.execute(rusqlite::params_from_iter(present_github_ids.iter()))?)
}

fn map_list(row: &rusqlite::Row<'_>) -> rusqlite::Result<ListRow> {
    Ok(ListRow {
        id: row.get(0)?,
        slug: row.get(1)?,
        name: row.get(2)?,
        description: row.get(3)?,
        is_private: row.get::<_, i64>(4)? != 0,
        github_id: row.get(5)?,
        status: row.get(6)?,
        repo_count: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

const LIST_SELECT: &str = r#"
SELECT l.id, l.slug, l.name, l.description, l.is_private, l.github_id, l.status,
       (SELECT COUNT(*) FROM membership m WHERE m.list_id = l.id) AS repo_count,
       l.created_at, l.updated_at
FROM list l
"#;

pub fn list_lists(conn: &Connection, include_tombstone: bool) -> Result<Vec<ListRow>> {
    let sql = if include_tombstone {
        format!("{LIST_SELECT} ORDER BY l.name COLLATE NOCASE")
    } else {
        format!("{LIST_SELECT} WHERE l.status != 'tombstone' ORDER BY l.name COLLATE NOCASE")
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], map_list)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn get_list(conn: &Connection, id: i64) -> Result<Option<ListRow>> {
    let sql = format!("{LIST_SELECT} WHERE l.id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    Ok(stmt.query_row(params![id], map_list).optional()?)
}

pub fn find_list_by_github_id(conn: &Connection, github_id: &str) -> Result<Option<ListRow>> {
    let sql = format!("{LIST_SELECT} WHERE l.github_id = ?1");
    let mut stmt = conn.prepare(&sql)?;
    Ok(stmt.query_row(params![github_id], map_list).optional()?)
}

pub fn find_list_by_name_ci(conn: &Connection, name: &str) -> Result<Option<ListRow>> {
    let sql = format!("{LIST_SELECT} WHERE lower(l.name) = lower(?1)");
    let mut stmt = conn.prepare(&sql)?;
    Ok(stmt.query_row(params![name], map_list).optional()?)
}

pub fn resolve_list(conn: &Connection, spec: &str) -> Result<ListRow> {
    let spec = spec.trim();
    if let Ok(id) = spec.parse::<i64>() {
        if let Some(row) = get_list(conn, id)? {
            return Ok(row);
        }
    }
    if spec.starts_with("UL_") {
        if let Some(row) = find_list_by_github_id(conn, spec)? {
            return Ok(row);
        }
    }
    let sql = format!("{LIST_SELECT} WHERE l.slug = ?1 OR lower(l.name) = lower(?1)");
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_row(params![spec], map_list)
        .optional()?
        .ok_or_else(|| AppError::new("not_found", format!("list not found: {spec}")).into())
}

pub fn create_list(
    conn: &Connection,
    name: &str,
    slug: Option<&str>,
    description: Option<&str>,
    is_private: bool,
) -> Result<ListRow> {
    let now = now_rfc3339();
    let taken = all_slugs(conn)?;
    let base = slug
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| slugify(s))
        .unwrap_or_else(|| slugify(name));
    if base.is_empty() {
        return Err(AppError::new("invalid", "list slug is empty").into());
    }
    let slug = unique_slug(&base, &taken);
    conn.execute(
        "INSERT INTO list (slug, name, description, is_private, status, created_at, updated_at)
         VALUES (?1,?2,?3,?4,'draft',?5,?5)",
        params![slug, name, description, is_private as i64, now],
    )?;
    let id = conn.last_insert_rowid();
    get_list(conn, id)?.ok_or_else(|| anyhow::anyhow!("list insert vanished"))
}

pub fn update_list(
    conn: &Connection,
    spec: &str,
    name: Option<&str>,
    description: Option<Option<&str>>,
    is_private: Option<bool>,
) -> Result<ListRow> {
    let list = resolve_list(conn, spec)?;
    let now = now_rfc3339();
    let new_name = name.unwrap_or(&list.name);
    let new_desc = match description {
        Some(d) => d.map(|s| s.to_string()),
        None => list.description.clone(),
    };
    let new_private = is_private.unwrap_or(list.is_private);
    let status = match list.status.as_str() {
        "synced" => "dirty",
        "tombstone" => {
            return Err(AppError::new("conflict", "cannot edit a tombstoned list")
                .hint("create a new list or wait until apply finishes")
                .into());
        }
        other => other,
    };
    conn.execute(
        "UPDATE list SET name=?1, description=?2, is_private=?3, status=?4, updated_at=?5 WHERE id=?6",
        params![new_name, new_desc, new_private as i64, status, now, list.id],
    )?;
    get_list(conn, list.id)?.ok_or_else(|| anyhow::anyhow!("list update vanished"))
}

pub fn delete_list(conn: &Connection, spec: &str) -> Result<ListRow> {
    let list = resolve_list(conn, spec)?;
    if list.github_id.is_none() && list.status == "draft" {
        conn.execute("DELETE FROM list WHERE id = ?1", params![list.id])?;
        return Ok(list);
    }
    let now = now_rfc3339();
    conn.execute(
        "UPDATE list SET status = 'tombstone', updated_at = ?1 WHERE id = ?2",
        params![now, list.id],
    )?;
    get_list(conn, list.id)?.ok_or_else(|| anyhow::anyhow!("list delete vanished"))
}

fn topics_from_json(raw: String) -> Vec<String> {
    serde_json::from_str(&raw).unwrap_or_default()
}

fn map_repo(row: &rusqlite::Row<'_>) -> rusqlite::Result<RepoRow> {
    let topics_raw: String = row.get(16)?;
    let lists_raw: String = row.get(17)?;
    let lists: Vec<String> = if lists_raw.is_empty() {
        Vec::new()
    } else {
        lists_raw.split('\u{1f}').map(|s| s.to_string()).collect()
    };
    Ok(RepoRow {
        id: row.get(0)?,
        node_id: row.get(1)?,
        name_with_owner: row.get(2)?,
        url: row.get(3)?,
        description: row.get(4)?,
        homepage_url: row.get(5)?,
        primary_language: row.get(6)?,
        license: row.get(7)?,
        stars: row.get(8)?,
        forks: row.get(9)?,
        is_private: row.get::<_, i64>(10)? != 0,
        is_archived: row.get::<_, i64>(11)? != 0,
        is_fork: row.get::<_, i64>(12)? != 0,
        pushed_at: row.get(13)?,
        updated_at: row.get(14)?,
        starred_at: row.get(15)?,
        topics: topics_from_json(topics_raw),
        lists,
    })
}

const REPO_SELECT: &str = r#"
SELECT r.id, r.node_id, r.name_with_owner, r.url, r.description, r.homepage_url,
       r.primary_language, r.license, r.stars, r.forks, r.is_private, r.is_archived,
       r.is_fork, r.pushed_at, r.updated_at, r.starred_at, r.topics_json,
       COALESCE((
           SELECT group_concat(l.slug, char(31))
           FROM membership m
           JOIN list l ON l.id = m.list_id
           WHERE m.repo_id = r.id AND l.status != 'tombstone'
       ), '') AS lists
FROM repo r
"#;

pub fn resolve_repo(conn: &Connection, spec: &str) -> Result<RepoRow> {
    let spec = spec.trim();
    let sql = if spec.parse::<i64>().is_ok() {
        format!("{REPO_SELECT} WHERE r.id = ?1")
    } else if spec.starts_with("R_") {
        format!("{REPO_SELECT} WHERE r.node_id = ?1")
    } else {
        format!(
            "{REPO_SELECT} WHERE r.name_with_owner = ?1 OR lower(r.name_with_owner) = lower(?1)"
        )
    };
    let mut stmt = conn.prepare(&sql)?;
    stmt.query_row(params![spec], map_repo)
        .optional()?
        .ok_or_else(|| AppError::new("not_found", format!("repository not found: {spec}")).into())
}

pub fn query_repos(
    conn: &Connection,
    list: Option<&ListRow>,
    unlisted: bool,
    query: Option<&str>,
    limit: usize,
) -> Result<Vec<RepoRow>> {
    let mut sql = REPO_SELECT.to_string();
    let mut clauses = Vec::new();
    if let Some(list) = list {
        clauses.push(format!(
            "EXISTS (SELECT 1 FROM membership m WHERE m.repo_id = r.id AND m.list_id = {})",
            list.id
        ));
    } else if unlisted {
        clauses.push(
            "NOT EXISTS (
                SELECT 1 FROM membership m
                JOIN list l ON l.id = m.list_id
                WHERE m.repo_id = r.id AND l.status != 'tombstone'
            )"
            .into(),
        );
    }
    if let Some(q) = query {
        let like = format!("'%{}%'", q.replace('\'', "''"));
        clauses.push(format!(
            "(r.name_with_owner LIKE {like} OR IFNULL(r.description,'') LIKE {like} OR r.topics_json LIKE {like})"
        ));
    }
    if !clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&clauses.join(" AND "));
    }
    sql.push_str(" ORDER BY r.starred_at DESC, r.stars DESC LIMIT ?1");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params![limit as i64], map_repo)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn all_repos(conn: &Connection) -> Result<Vec<RepoRow>> {
    let sql = format!("{REPO_SELECT} ORDER BY r.name_with_owner");
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map([], map_repo)?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn repo_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM repo", [], |r| r.get(0))?)
}

pub fn membership_slugs(conn: &Connection, repo_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT l.slug FROM membership m JOIN list l ON l.id = m.list_id
         WHERE m.repo_id = ?1 AND l.status != 'tombstone' ORDER BY l.slug",
    )?;
    let rows = stmt.query_map(params![repo_id], |r| r.get(0))?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn set_membership(
    conn: &Connection,
    repo_id: i64,
    list_ids: &[i64],
    source: &str,
) -> Result<()> {
    conn.execute(
        "DELETE FROM membership WHERE repo_id = ?1",
        params![repo_id],
    )?;
    for list_id in list_ids {
        add_membership(conn, repo_id, *list_id, source)?;
    }
    Ok(())
}

pub fn stats(conn: &Connection) -> Result<Value> {
    let repos: i64 = repo_count(conn)?;
    let lists: i64 = conn.query_row(
        "SELECT COUNT(*) FROM list WHERE status != 'tombstone'",
        [],
        |r| r.get(0),
    )?;
    let unlisted: i64 = conn.query_row(
        "SELECT COUNT(*) FROM repo r WHERE NOT EXISTS (
            SELECT 1 FROM membership m JOIN list l ON l.id = m.list_id
            WHERE m.repo_id = r.id AND l.status != 'tombstone'
        )",
        [],
        |r| r.get(0),
    )?;
    Ok(serde_json::json!({
        "repos": repos,
        "lists": lists,
        "unlisted": unlisted,
    }))
}

pub fn insert_proposal(
    conn: &Connection,
    id: &str,
    notes: Option<&str>,
    source_path: Option<&str>,
    raw_json: &str,
) -> Result<()> {
    let now = now_rfc3339();
    conn.execute(
        "INSERT INTO proposal (id, created_at, notes, status, source_path, raw_json)
         VALUES (?1,?2,?3,'draft',?4,?5)",
        params![id, now, notes, source_path, raw_json],
    )?;
    set_meta(conn, "last_proposal_id", id)?;
    Ok(())
}

pub fn replace_proposal(
    conn: &Connection,
    id: &str,
    notes: Option<&str>,
    source_path: Option<&str>,
    raw_json: &str,
) -> Result<()> {
    conn.execute("DELETE FROM proposal WHERE id = ?1", params![id])?;
    insert_proposal(conn, id, notes, source_path, raw_json)
}

pub fn get_proposal(conn: &Connection, id: &str) -> Result<Option<ProposalRow>> {
    Ok(conn
        .query_row(
            "SELECT id, created_at, notes, status, source_path, raw_json FROM proposal WHERE id = ?1",
            params![id],
            |r| {
                Ok(ProposalRow {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    notes: r.get(2)?,
                    status: r.get(3)?,
                    source_path: r.get(4)?,
                    raw_json: r.get(5)?,
                })
            },
        )
        .optional()?)
}

pub fn latest_proposal(conn: &Connection) -> Result<Option<ProposalRow>> {
    if let Some(id) = get_meta(conn, "last_proposal_id")? {
        if let Some(row) = get_proposal(conn, &id)? {
            return Ok(Some(row));
        }
    }
    Ok(conn
        .query_row(
            "SELECT id, created_at, notes, status, source_path, raw_json FROM proposal ORDER BY created_at DESC LIMIT 1",
            [],
            |r| {
                Ok(ProposalRow {
                    id: r.get(0)?,
                    created_at: r.get(1)?,
                    notes: r.get(2)?,
                    status: r.get(3)?,
                    source_path: r.get(4)?,
                    raw_json: r.get(5)?,
                })
            },
        )
        .optional()?)
}

pub fn mark_proposal_applied(conn: &Connection, id: &str) -> Result<()> {
    conn.execute(
        "UPDATE proposal SET status = 'applied' WHERE id = ?1",
        params![id],
    )?;
    Ok(())
}
