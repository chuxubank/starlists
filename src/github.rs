use anyhow::{Context, Result};
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, USER_AGENT};
use serde::Serialize;
use serde_json::{json, Value};

use crate::db::RepoRow;
use crate::output::AppError;

const STARRED_QUERY: &str = r#"
query($after: String) {
  viewer {
    login
    starredRepositories(first: 50, after: $after, orderBy: {field: STARRED_AT, direction: DESC}) {
      pageInfo { hasNextPage endCursor }
      edges {
        starredAt
        node {
          id
          databaseId
          nameWithOwner
          url
          description
          homepageUrl
          primaryLanguage { name }
          licenseInfo { spdxId }
          stargazerCount
          forkCount
          isPrivate
          isArchived
          isFork
          pushedAt
          updatedAt
          repositoryTopics(first: 20) { nodes { topic { name } } }
        }
      }
    }
  }
}
"#;

const LISTS_QUERY: &str = r#"
query($after: String) {
  viewer {
    lists(first: 32, after: $after) {
      pageInfo { hasNextPage endCursor }
      nodes {
        id
        name
        slug
        description
        isPrivate
      }
    }
  }
}
"#;

const LIST_ITEMS_QUERY: &str = r#"
query($id: ID!, $after: String) {
  node(id: $id) {
    ... on UserList {
      items(first: 50, after: $after) {
        pageInfo { hasNextPage endCursor }
        nodes {
          ... on Repository {
            id
            databaseId
            nameWithOwner
            url
          }
        }
      }
    }
  }
}
"#;

pub const GITHUB_LIST_CAP: usize = 32;

#[derive(Debug, Clone)]
pub struct RemoteList {
    pub github_id: String,
    pub name: String,
    pub slug: Option<String>,
    pub description: Option<String>,
    pub is_private: bool,
    pub repos: Vec<RepoRow>,
}

pub struct GitHub {
    client: Client,
    url: String,
}

impl GitHub {
    pub fn new(token: &str, url: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}"))
                .context("invalid token characters")?,
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_static("stars-cli (github-stars)"),
        );
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("application/vnd.github+json"),
        );
        let client = Client::builder().default_headers(headers).build()?;
        Ok(Self {
            client,
            url: url.to_string(),
        })
    }

    pub fn graphql(&self, query: &str, variables: Value) -> Result<Value> {
        if looks_like_mutation(query) {
            // caller must opt in via request --write; snapshot/apply call mutations explicitly
        }
        let body = self.graphql_once(query, &variables)?;
        if body.get("data").is_none() {
            if let Some(message) = body.get("message").and_then(Value::as_str) {
                return Err(AppError::new("github_http", message.to_string()).into());
            }
        }
        if let Some(errors) = body.get("errors").and_then(Value::as_array) {
            if !errors.is_empty() {
                let message = errors
                    .iter()
                    .filter_map(|e| e.get("message").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("; ");
                let code = if message.to_lowercase().contains("scope") {
                    "auth_scope"
                } else {
                    "github_graphql"
                };
                let err = AppError::new(code, message);
                let err = if code == "auth_scope" {
                    err.hint("run `gh auth refresh -h github.com -s user` for Star List writes")
                } else {
                    err
                };
                return Err(err.into());
            }
        }
        body.get("data")
            .cloned()
            .ok_or_else(|| AppError::new("github_graphql", "GraphQL response missing data").into())
    }

    fn graphql_once(&self, query: &str, variables: &Value) -> Result<Value> {
        const ATTEMPTS: u32 = 4;
        let mut last_err = None;
        for attempt in 1..=ATTEMPTS {
            let resp = self
                .client
                .post(&self.url)
                .json(&json!({ "query": query, "variables": variables }))
                .send()
                .context("GitHub GraphQL request")?;
            let status = resp.status();
            let bytes = resp.bytes().context("read GitHub GraphQL body")?;
            match decode_graphql_body(status.as_u16(), &bytes) {
                Ok(body) => return Ok(body),
                Err(err) => {
                    let retryable = status.as_u16() == 502
                        || status.as_u16() == 503
                        || status.as_u16() == 429
                        || bytes.is_empty();
                    if retryable && attempt < ATTEMPTS {
                        let wait = std::time::Duration::from_millis(400 * 2u64.pow(attempt - 1));
                        eprintln!(
                            "GitHub HTTP {} (attempt {attempt}/{ATTEMPTS}), retrying…",
                            status.as_u16()
                        );
                        std::thread::sleep(wait);
                        last_err = Some(err);
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("GitHub GraphQL failed")))
    }

    pub fn fetch_starred(&self) -> Result<(String, Vec<RepoRow>)> {
        let mut after: Option<String> = None;
        let mut repos = Vec::new();
        let mut login = String::new();
        loop {
            let data = self.graphql(STARRED_QUERY, json!({ "after": after }))?;
            login = data["viewer"]["login"]
                .as_str()
                .unwrap_or(&login)
                .to_string();
            let conn = &data["viewer"]["starredRepositories"];
            if let Some(edges) = conn["edges"].as_array() {
                for edge in edges {
                    if let Some(repo) = parse_repo(&edge["node"], edge["starredAt"].as_str()) {
                        repos.push(repo);
                    }
                }
            }
            let page = &conn["pageInfo"];
            if page["hasNextPage"].as_bool().unwrap_or(false) {
                after = page["endCursor"].as_str().map(|s| s.to_string());
            } else {
                break;
            }
        }
        Ok((login, repos))
    }

    pub fn fetch_lists(&self) -> Result<Vec<RemoteList>> {
        let mut after: Option<String> = None;
        let mut lists = Vec::new();
        loop {
            let data = self.graphql(LISTS_QUERY, json!({ "after": after }))?;
            let conn = &data["viewer"]["lists"];
            if conn.is_null() {
                return Err(AppError::new(
                    "github_graphql",
                    "viewer.lists is not available on this account",
                )
                .hint("Star Lists are a GitHub preview feature; confirm they appear on github.com?tab=stars")
                .into());
            }
            if let Some(nodes) = conn["nodes"].as_array() {
                for node in nodes {
                    let github_id = node["id"].as_str().unwrap_or_default().to_string();
                    if github_id.is_empty() {
                        continue;
                    }
                    let name = node["name"].as_str().unwrap_or("untitled").to_string();
                    eprintln!("  list {name}");
                    let repos = self.fetch_list_items(&github_id)?;
                    lists.push(RemoteList {
                        github_id,
                        name,
                        slug: node["slug"].as_str().map(|s| s.to_string()),
                        description: node["description"].as_str().map(|s| s.to_string()),
                        is_private: node["isPrivate"].as_bool().unwrap_or(true),
                        repos,
                    });
                }
            }
            let page = &conn["pageInfo"];
            if page["hasNextPage"].as_bool().unwrap_or(false) {
                after = page["endCursor"].as_str().map(|s| s.to_string());
            } else {
                break;
            }
        }
        Ok(lists)
    }

    fn fetch_list_items(&self, github_id: &str) -> Result<Vec<RepoRow>> {
        let mut repos = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let data =
                self.graphql(LIST_ITEMS_QUERY, json!({ "id": github_id, "after": after }))?;
            let items = &data["node"]["items"];
            repos.extend(parse_item_repos(items));
            let page = &items["pageInfo"];
            if page["hasNextPage"].as_bool().unwrap_or(false) {
                after = page["endCursor"].as_str().map(|s| s.to_string());
            } else {
                break;
            }
        }
        Ok(repos)
    }

    pub fn create_list(
        &self,
        name: &str,
        description: Option<&str>,
        is_private: bool,
    ) -> Result<CreatedList> {
        let data = self.graphql(
            r#"
            mutation($name: String!, $description: String, $isPrivate: Boolean) {
              createUserList(input: {name: $name, description: $description, isPrivate: $isPrivate}) {
                list { id name slug }
              }
            }
            "#,
            json!({
                "name": name,
                "description": description,
                "isPrivate": is_private
            }),
        )?;
        let list = &data["createUserList"]["list"];
        Ok(CreatedList {
            github_id: list["id"]
                .as_str()
                .ok_or_else(|| AppError::new("github_graphql", "createUserList returned no id"))?
                .to_string(),
            name: list["name"].as_str().unwrap_or(name).to_string(),
            slug: list["slug"].as_str().map(|s| s.to_string()),
        })
    }

    pub fn update_list(
        &self,
        github_id: &str,
        name: Option<&str>,
        description: Option<&str>,
        is_private: Option<bool>,
    ) -> Result<()> {
        self.graphql(
            r#"
            mutation($listId: ID!, $name: String, $description: String, $isPrivate: Boolean) {
              updateUserList(input: {listId: $listId, name: $name, description: $description, isPrivate: $isPrivate}) {
                list { id }
              }
            }
            "#,
            json!({
                "listId": github_id,
                "name": name,
                "description": description,
                "isPrivate": is_private
            }),
        )?;
        Ok(())
    }

    pub fn delete_list(&self, github_id: &str) -> Result<()> {
        self.graphql(
            r#"
            mutation($listId: ID!) {
              deleteUserList(input: {listId: $listId}) { list { id } }
            }
            "#,
            json!({ "listId": github_id }),
        )?;
        Ok(())
    }

    pub fn set_item_lists(&self, repo_node_id: &str, list_ids: &[String]) -> Result<Vec<String>> {
        let data = self.graphql(
            r#"
            mutation($itemId: ID!, $listIds: [ID!]!) {
              updateUserListsForItem(input: {itemId: $itemId, listIds: $listIds}) {
                lists { id name }
              }
            }
            "#,
            json!({ "itemId": repo_node_id, "listIds": list_ids }),
        )?;
        Ok(data["updateUserListsForItem"]["lists"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l["name"].as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default())
    }
}

#[derive(Debug, Serialize)]
pub struct CreatedList {
    pub github_id: String,
    pub name: String,
    pub slug: Option<String>,
}

pub fn decode_graphql_body(status: u16, bytes: &[u8]) -> Result<Value> {
    if bytes.is_empty() {
        return Err(AppError::new(
            "github_http",
            format!("GitHub GraphQL returned empty body (HTTP {status})"),
        )
        .into());
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(body) => Ok(body),
        Err(_) => {
            let prefix = String::from_utf8_lossy(bytes);
            let prefix: String = prefix.chars().take(80).collect();
            let hint = if prefix.contains("502") {
                Some("query was too heavy or GitHub timed out; retry with a smaller page")
            } else {
                None
            };
            let mut err = AppError::new(
                "github_http",
                format!("GitHub GraphQL HTTP {status}, non-JSON body: {prefix}"),
            );
            if let Some(hint) = hint {
                err = err.hint(hint);
            }
            Err(err.into())
        }
    }
}

pub fn looks_like_mutation(query: &str) -> bool {
    query
        .split_whitespace()
        .next()
        .is_some_and(|w| w.eq_ignore_ascii_case("mutation"))
}

fn parse_item_repos(items: &Value) -> Vec<RepoRow> {
    items["nodes"]
        .as_array()
        .map(|nodes| nodes.iter().filter_map(|n| parse_repo(n, None)).collect())
        .unwrap_or_default()
}

pub fn parse_repo(node: &Value, starred_at: Option<&str>) -> Option<RepoRow> {
    let id = node["databaseId"].as_i64()?;
    let node_id = node["id"].as_str()?.to_string();
    let name_with_owner = node["nameWithOwner"].as_str()?.to_string();
    let topics = node["repositoryTopics"]["nodes"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|n| n["topic"]["name"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    Some(RepoRow {
        id,
        node_id,
        name_with_owner,
        url: node["url"].as_str().unwrap_or_default().to_string(),
        description: node["description"].as_str().map(|s| s.to_string()),
        homepage_url: node["homepageUrl"].as_str().map(|s| s.to_string()),
        primary_language: node["primaryLanguage"]["name"]
            .as_str()
            .map(|s| s.to_string()),
        license: node["licenseInfo"]["spdxId"]
            .as_str()
            .map(|s| s.to_string()),
        stars: node["stargazerCount"].as_i64(),
        forks: node["forkCount"].as_i64(),
        is_private: node["isPrivate"].as_bool().unwrap_or(false),
        is_archived: node["isArchived"].as_bool().unwrap_or(false),
        is_fork: node["isFork"].as_bool().unwrap_or(false),
        pushed_at: node["pushedAt"].as_str().map(|s| s.to_string()),
        updated_at: node["updatedAt"].as_str().map(|s| s.to_string()),
        starred_at: starred_at.map(|s| s.to_string()),
        topics,
        lists: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_starred_edge() {
        let node = json!({
            "id": "R_kgDOAbcd",
            "databaseId": 42,
            "nameWithOwner": "foo/bar",
            "url": "https://github.com/foo/bar",
            "description": "demo",
            "homepageUrl": null,
            "primaryLanguage": {"name": "Rust"},
            "licenseInfo": {"spdxId": "MIT"},
            "stargazerCount": 10,
            "forkCount": 1,
            "isPrivate": false,
            "isArchived": false,
            "isFork": false,
            "pushedAt": "2026-01-01T00:00:00Z",
            "updatedAt": "2026-01-01T00:00:00Z",
            "repositoryTopics": {"nodes": [{"topic": {"name": "cli"}}]}
        });
        let repo = parse_repo(&node, Some("2026-02-01T00:00:00Z")).unwrap();
        assert_eq!(repo.name_with_owner, "foo/bar");
        assert_eq!(repo.topics, vec!["cli"]);
        assert_eq!(repo.starred_at.as_deref(), Some("2026-02-01T00:00:00Z"));
    }

    #[test]
    fn mutation_detect() {
        assert!(looks_like_mutation("mutation { foo }"));
        assert!(!looks_like_mutation("query { viewer { login } }"));
    }

    #[test]
    fn decode_502_html_is_readable() {
        let html = b"<html>\r\n<head><title>502 Bad Gateway</title></head>\r\n<body>\r\n<center><h1>502 Bad Gateway</h1></center>\r\n</body>\r\n</html>\r\n";
        let err = decode_graphql_body(502, html).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("502"), "{msg}");
        assert!(msg.contains("non-JSON"), "{msg}");
    }

    #[test]
    fn decode_empty_body() {
        let err = decode_graphql_body(200, b"").unwrap_err();
        assert!(format!("{err:#}").contains("empty body"));
    }
}
