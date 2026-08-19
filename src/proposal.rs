use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::db::{self, ListRow};
use crate::github::GITHUB_LIST_CAP;
use crate::ids::slugify;
use crate::output::AppError;

pub const PROPOSAL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProposalFile {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub lists: ListChanges,
    #[serde(default)]
    pub memberships: Vec<MembershipSet>,
}

fn default_version() -> u32 {
    PROPOSAL_VERSION
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListChanges {
    #[serde(default)]
    pub create: Vec<ListCreate>,
    #[serde(default)]
    pub rename: Vec<ListRename>,
    #[serde(default)]
    pub update: Vec<ListUpdate>,
    #[serde(default)]
    pub merge: Vec<ListMerge>,
    #[serde(default)]
    pub delete: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListCreate {
    pub slug: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_private: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListRename {
    pub slug: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListUpdate {
    pub slug: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub is_private: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListMerge {
    pub from: Vec<String>,
    pub into: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MembershipSet {
    pub repo: String,
    pub lists: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub id: String,
    pub notes: Option<String>,
    pub list_cap: ListCap,
    pub list_ops: Vec<ListOp>,
    pub memberships: Vec<MembershipPlan>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListCap {
    pub current_remote: usize,
    pub after_apply: usize,
    pub max: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ListOp {
    Create {
        slug: String,
        name: String,
        description: Option<String>,
        is_private: bool,
    },
    Update {
        slug: String,
        github_id: Option<String>,
        name: Option<String>,
        description: Option<String>,
        is_private: Option<bool>,
    },
    Delete {
        slug: String,
        github_id: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct MembershipPlan {
    pub repo: String,
    pub repo_node_id: String,
    pub lists: Vec<String>,
    pub list_github_ids: Vec<String>,
}

pub fn parse_proposal(raw: &str) -> Result<ProposalFile> {
    let parsed: ProposalFile = serde_json::from_str(raw)
        .map_err(|e| AppError::new("invalid", format!("proposal JSON: {e}")))?;
    if parsed.version != PROPOSAL_VERSION {
        return Err(AppError::new(
            "invalid",
            format!(
                "unsupported proposal version {}, expected {PROPOSAL_VERSION}",
                parsed.version
            ),
        )
        .into());
    }
    Ok(parsed)
}

pub fn example_proposal() -> ProposalFile {
    ProposalFile {
        version: PROPOSAL_VERSION,
        id: None,
        notes: Some("Replace this file after classifying the export corpus.".into()),
        lists: ListChanges {
            create: vec![ListCreate {
                slug: Some("web-mapping".into()),
                name: "Web Mapping".into(),
                description: Some("Browser maps, tiles, and Web GIS.".into()),
                is_private: Some(true),
            }],
            ..Default::default()
        },
        memberships: vec![MembershipSet {
            repo: "owner/example".into(),
            lists: vec!["web-mapping".into()],
        }],
    }
}

pub fn compile_plan(conn: &Connection, id: &str, proposal: &ProposalFile) -> Result<Plan> {
    let lists = db::list_lists(conn, true)?;
    let by_slug: HashMap<String, ListRow> =
        lists.iter().cloned().map(|l| (l.slug.clone(), l)).collect();
    let mut warnings = Vec::new();

    let mut creates: BTreeMap<String, ListCreate> = BTreeMap::new();
    for create in &proposal.lists.create {
        let slug = create
            .slug
            .as_deref()
            .map(slugify)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| slugify(&create.name));
        if slug.is_empty() {
            return Err(AppError::new("invalid", "proposal create list has empty slug").into());
        }
        creates.insert(slug, create.clone());
    }

    let mut renames: HashMap<String, String> = HashMap::new();
    for rename in &proposal.lists.rename {
        renames.insert(rename.slug.clone(), rename.name.clone());
    }

    let mut updates: HashMap<String, ListUpdate> = HashMap::new();
    for update in &proposal.lists.update {
        updates.insert(update.slug.clone(), update.clone());
    }

    let mut delete: BTreeSet<String> = proposal.lists.delete.iter().cloned().collect();
    let mut memberships = proposal.memberships.clone();

    for merge in &proposal.lists.merge {
        let into = merge.into.clone();
        if !by_slug.contains_key(&into) && !creates.contains_key(&into) {
            creates.insert(
                into.clone(),
                ListCreate {
                    slug: Some(into.clone()),
                    name: into.clone(),
                    description: None,
                    is_private: Some(true),
                },
            );
        }
        for from in &merge.from {
            if from == &into {
                continue;
            }
            delete.insert(from.clone());
            for membership in &mut memberships {
                if membership.lists.iter().any(|s| s == from) {
                    membership.lists.retain(|s| s != from);
                    if !membership.lists.iter().any(|s| s == &into) {
                        membership.lists.push(into.clone());
                    }
                }
            }
        }
    }

    let mut list_ops = Vec::new();
    for (slug, create) in &creates {
        if by_slug
            .get(slug)
            .and_then(|l| l.github_id.as_ref())
            .is_some()
        {
            warnings.push(format!("create skipped, list already on GitHub: {slug}"));
            continue;
        }
        list_ops.push(ListOp::Create {
            slug: slug.clone(),
            name: create.name.clone(),
            description: create.description.clone(),
            is_private: create.is_private.unwrap_or(true),
        });
    }

    for (slug, name) in &renames {
        let existing = by_slug.get(slug);
        if existing.is_none() && !creates.contains_key(slug) {
            return Err(
                AppError::new("not_found", format!("rename target missing: {slug}")).into(),
            );
        }
        list_ops.push(ListOp::Update {
            slug: slug.clone(),
            github_id: existing.and_then(|l| l.github_id.clone()),
            name: Some(name.clone()),
            description: None,
            is_private: None,
        });
    }

    for (slug, update) in &updates {
        let existing = by_slug.get(slug);
        if existing.is_none() && !creates.contains_key(slug) {
            return Err(
                AppError::new("not_found", format!("update target missing: {slug}")).into(),
            );
        }
        list_ops.push(ListOp::Update {
            slug: slug.clone(),
            github_id: existing.and_then(|l| l.github_id.clone()),
            name: None,
            description: update.description.clone(),
            is_private: update.is_private,
        });
    }

    for slug in &delete {
        let existing = by_slug.get(slug);
        list_ops.push(ListOp::Delete {
            slug: slug.clone(),
            github_id: existing.and_then(|l| l.github_id.clone()),
        });
    }

    let mut membership_plans = Vec::new();
    for item in &memberships {
        let repo = db::resolve_repo(conn, &item.repo)?;
        let mut slugs = item.lists.clone();
        slugs.sort();
        slugs.dedup();
        let mut github_ids = Vec::new();
        for slug in &slugs {
            if let Some(list) = by_slug.get(slug) {
                if let Some(gid) = &list.github_id {
                    github_ids.push(gid.clone());
                } else {
                    github_ids.push(format!("pending:{slug}"));
                }
            } else if creates.contains_key(slug) {
                github_ids.push(format!("pending:{slug}"));
            } else {
                return Err(AppError::new(
                    "not_found",
                    format!("membership refers to unknown list {slug} for {}", item.repo),
                )
                .into());
            }
        }
        membership_plans.push(MembershipPlan {
            repo: repo.name_with_owner,
            repo_node_id: repo.node_id,
            lists: slugs,
            list_github_ids: github_ids,
        });
    }

    let current_remote = lists
        .iter()
        .filter(|l| l.github_id.is_some() && l.status != "tombstone")
        .count();
    let remote_deletes = list_ops
        .iter()
        .filter(|op| match op {
            ListOp::Delete {
                slug,
                github_id: Some(_),
            } => by_slug
                .get(slug)
                .is_some_and(|list| list.status != "tombstone"),
            _ => false,
        })
        .count();
    let remote_creates = list_ops
        .iter()
        .filter(|op| matches!(op, ListOp::Create { .. }))
        .count();
    let after_apply = current_remote
        .saturating_add(remote_creates)
        .saturating_sub(remote_deletes);
    if after_apply > GITHUB_LIST_CAP {
        warnings.push(format!(
            "apply would leave {after_apply} GitHub lists; cap is {GITHUB_LIST_CAP}"
        ));
    }

    Ok(Plan {
        id: id.to_string(),
        notes: proposal.notes.clone(),
        list_cap: ListCap {
            current_remote,
            after_apply,
            max: GITHUB_LIST_CAP,
        },
        list_ops,
        memberships: membership_plans,
        warnings,
    })
}

pub fn apply_proposal_locally(conn: &Connection, proposal: &ProposalFile) -> Result<Vec<String>> {
    let mut log = Vec::new();
    for create in &proposal.lists.create {
        let list = db::create_list(
            conn,
            &create.name,
            create.slug.as_deref(),
            create.description.as_deref(),
            create.is_private.unwrap_or(true),
        )?;
        log.push(format!("draft list {}", list.slug));
    }
    for rename in &proposal.lists.rename {
        db::update_list(conn, &rename.slug, Some(&rename.name), None, None)?;
        log.push(format!("rename {}", rename.slug));
    }
    for update in &proposal.lists.update {
        db::update_list(
            conn,
            &update.slug,
            None,
            update.description.as_deref().map(Some),
            update.is_private,
        )?;
        log.push(format!("update {}", update.slug));
    }
    for merge in &proposal.lists.merge {
        if db::resolve_list(conn, &merge.into).is_err() {
            db::create_list(conn, &merge.into, Some(&merge.into), None, true)?;
        }
        let into = db::resolve_list(conn, &merge.into)?;
        for from_spec in &merge.from {
            if from_spec == &merge.into {
                continue;
            }
            let from = db::resolve_list(conn, from_spec)?;
            let repos = db::query_repos(conn, Some(&from), false, None, 10_000)?;
            for repo in repos {
                let mut slugs = db::membership_slugs(conn, repo.id)?;
                slugs.retain(|s| s != &from.slug);
                if !slugs.iter().any(|s| s == &into.slug) {
                    slugs.push(into.slug.clone());
                }
                let ids = slugs
                    .iter()
                    .map(|s| db::resolve_list(conn, s).map(|l| l.id))
                    .collect::<Result<Vec<_>>>()?;
                db::set_membership(conn, repo.id, &ids, "proposal")?;
            }
            db::delete_list(conn, &from.slug)?;
            log.push(format!("merge {} -> {}", from.slug, into.slug));
        }
    }
    for slug in &proposal.lists.delete {
        db::delete_list(conn, slug)?;
        log.push(format!("delete {slug}"));
    }
    for item in &proposal.memberships {
        let repo = db::resolve_repo(conn, &item.repo)?;
        let ids = item
            .lists
            .iter()
            .map(|s| db::resolve_list(conn, s).map(|l| l.id))
            .collect::<Result<Vec<_>>>()?;
        db::set_membership(conn, repo.id, &ids, "proposal")?;
        log.push(format!("membership {}", repo.name_with_owner));
    }
    Ok(log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, RepoRow};

    fn seed_repo(conn: &Connection) {
        let repo = RepoRow {
            id: 1,
            node_id: "R_kgDOTest".into(),
            name_with_owner: "foo/bar".into(),
            url: "https://github.com/foo/bar".into(),
            description: Some("demo".into()),
            homepage_url: None,
            primary_language: Some("Rust".into()),
            license: Some("MIT".into()),
            stars: Some(10),
            forks: Some(1),
            is_private: false,
            is_archived: false,
            is_fork: false,
            pushed_at: None,
            updated_at: None,
            starred_at: None,
            topics: vec!["cli".into()],
            lists: vec![],
        };
        db::upsert_repo(conn, &repo, "2026-01-01T00:00:00Z").unwrap();
    }

    #[test]
    fn compile_plan_multi_list() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("t.db")).unwrap();
        seed_repo(&conn);
        let proposal = ProposalFile {
            version: 1,
            id: Some("PLAN_TEST".into()),
            notes: None,
            lists: ListChanges {
                create: vec![
                    ListCreate {
                        slug: Some("emacs".into()),
                        name: "Emacs".into(),
                        description: None,
                        is_private: Some(true),
                    },
                    ListCreate {
                        slug: Some("cli".into()),
                        name: "CLI".into(),
                        description: None,
                        is_private: Some(true),
                    },
                ],
                ..Default::default()
            },
            memberships: vec![MembershipSet {
                repo: "foo/bar".into(),
                lists: vec!["emacs".into(), "cli".into()],
            }],
        };
        let plan = compile_plan(&conn, "PLAN_TEST", &proposal).unwrap();
        assert_eq!(plan.list_ops.len(), 2);
        assert_eq!(plan.memberships[0].lists, vec!["cli", "emacs"]);
        assert!(plan.memberships[0]
            .list_github_ids
            .iter()
            .all(|id| id.starts_with("pending:")));
    }

    #[test]
    fn apply_locally_sets_full_membership() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("t.db")).unwrap();
        seed_repo(&conn);
        let proposal = parse_proposal(
            r#"{
              "version": 1,
              "lists": { "create": [{ "slug": "emacs", "name": "Emacs" }, { "slug": "cli", "name": "CLI" }] },
              "memberships": [{ "repo": "foo/bar", "lists": ["emacs", "cli"] }]
            }"#,
        )
        .unwrap();
        apply_proposal_locally(&conn, &proposal).unwrap();
        let slugs = db::membership_slugs(&conn, 1).unwrap();
        assert_eq!(slugs, vec!["cli", "emacs"]);
    }

    #[test]
    fn list_cap_ignores_already_tombstoned_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let conn = db::open(&dir.path().join("t.db")).unwrap();
        seed_repo(&conn);
        db::create_list(&conn, "Keep", Some("keep"), None, true).unwrap();
        let gone = db::create_list(&conn, "Gone", Some("gone"), None, true).unwrap();
        conn.execute(
            "UPDATE list SET github_id = 'UL_keep', status = 'synced' WHERE slug = 'keep'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE list SET github_id = 'UL_gone', status = 'tombstone' WHERE id = ?1",
            rusqlite::params![gone.id],
        )
        .unwrap();
        let proposal = parse_proposal(
            r#"{
              "version": 1,
              "lists": { "delete": ["gone"] },
              "memberships": []
            }"#,
        )
        .unwrap();
        let plan = compile_plan(&conn, "PLAN_CAP", &proposal).unwrap();
        assert_eq!(plan.list_cap.current_remote, 1);
        assert_eq!(plan.list_cap.after_apply, 1);
    }
}
