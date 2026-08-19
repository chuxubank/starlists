use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::output::AppError;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub db_file: PathBuf,
}

impl Paths {
    pub fn resolve(config: Option<PathBuf>, db: Option<PathBuf>) -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        let config_dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("stars");
        let data_dir = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"))
            .join("stars");
        let config_file = config.unwrap_or_else(|| config_dir.join("config.toml"));
        let db_file = db.unwrap_or_else(|| data_dir.join("stars.db"));
        Ok(Self {
            config_file,
            data_dir,
            db_file,
        })
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    pub github_token: Option<String>,
    pub github_host: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub paths: Paths,
    pub token: Option<String>,
    pub token_source: TokenSource,
    pub api_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenSource {
    Env,
    Config,
    GhCli,
    Missing,
}

impl TokenSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Env => "env",
            Self::Config => "config",
            Self::GhCli => "gh",
            Self::Missing => "missing",
        }
    }
}

impl AppConfig {
    pub fn load(paths: Paths) -> Result<Self> {
        let file = if paths.config_file.exists() {
            let text = fs::read_to_string(&paths.config_file)
                .with_context(|| format!("read config {}", paths.config_file.display()))?;
            toml::from_str(&text).context("parse config.toml")?
        } else {
            ConfigFile::default()
        };

        let (token, token_source) = resolve_token(&file);
        let host = file
            .github_host
            .clone()
            .unwrap_or_else(|| "github.com".to_string());
        let api_url = if host == "github.com" {
            "https://api.github.com/graphql".to_string()
        } else {
            format!("https://{host}/api/graphql")
        };

        Ok(Self {
            paths,
            token,
            token_source,
            api_url,
        })
    }

    pub fn require_token(&self) -> Result<&str> {
        self.token.as_deref().ok_or_else(|| {
            AppError::new(
                "auth_missing",
                "no GitHub token found (GITHUB_TOKEN, GH_TOKEN, STARS_GITHUB_TOKEN, config, or gh auth token)",
            )
            .hint("run `stars init` or `gh auth login`, and `gh auth refresh -s user` for list writes")
            .into()
        })
    }
}

fn resolve_token(file: &ConfigFile) -> (Option<String>, TokenSource) {
    for key in ["STARS_GITHUB_TOKEN", "GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(value) = std::env::var(key) {
            if !value.trim().is_empty() {
                return (Some(value), TokenSource::Env);
            }
        }
    }
    if let Some(token) = file
        .github_token
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return (Some(token), TokenSource::Config);
    }
    if let Some(token) = gh_auth_token() {
        return (Some(token), TokenSource::GhCli);
    }
    (None, TokenSource::Missing)
}

fn gh_auth_token() -> Option<String> {
    let output = Command::new("gh").args(["auth", "token"]).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let token = String::from_utf8(output.stdout).ok()?;
    let token = token.trim().to_string();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

pub fn write_init(paths: &Paths) -> Result<bool> {
    fs::create_dir_all(paths.config_file.parent().unwrap_or(Path::new(".")))?;
    fs::create_dir_all(&paths.data_dir)?;
    if paths.config_file.exists() {
        return Ok(false);
    }
    let sample = r#"# Optional. Prefer GITHUB_TOKEN / GH_TOKEN / `gh auth token`.
# github_token = ""
# github_host = "github.com"
"#;
    fs::write(&paths.config_file, sample)?;
    Ok(true)
}
