use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Format {
    Table,
    Json,
    Sexp,
}

#[derive(Parser, Debug)]
#[command(
    name = "stars",
    version,
    about = "Starlists: organize GitHub stars with official Star Lists",
    after_help = "JSON policy: --json prints a {ok, data|error} envelope on stdout. Progress goes to stderr. Tokens are never printed."
)]
pub struct Cli {
    /// Machine-readable JSON envelope on stdout
    #[arg(long, global = true)]
    pub json: bool,

    /// Emacs-readable s-expression on stdout
    #[arg(long, global = true)]
    pub sexp: bool,

    /// Override the SQLite database path
    #[arg(long, global = true, env = "STARS_DB")]
    pub db: Option<PathBuf>,

    /// Override the config file path
    #[arg(long, global = true, env = "STARS_CONFIG")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    pub fn format(&self) -> Format {
        if self.json {
            Format::Json
        } else if self.sexp {
            Format::Sexp
        } else {
            Format::Table
        }
    }
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check config, auth, database, and GitHub reachability
    Doctor,
    /// Create the config file and data directory
    Init,
    /// Pull starred repos and official Lists into the local database
    Snapshot {
        /// Skip official Lists (stars only)
        #[arg(long)]
        stars_only: bool,
    },
    /// Official / local Star Lists
    Lists {
        #[command(subcommand)]
        command: Option<ListsCommand>,
    },
    /// Starred repositories
    Repos {
        #[command(subcommand)]
        command: Option<ReposCommand>,
    },
    /// Change a repository's local list membership (does not write GitHub)
    Assign {
        /// owner/repo, numeric id, or GraphQL node id
        repo: String,
        /// List slug, name, or id to add
        #[arg(long = "add")]
        add: Vec<String>,
        /// List slug, name, or id to remove
        #[arg(long = "remove")]
        remove: Vec<String>,
        /// Replace membership with these lists (full set)
        #[arg(long = "set", conflicts_with_all = ["add", "remove"])]
        set: Vec<String>,
    },
    /// Export a corpus JSON for an external classifier
    Export {
        /// Write to this path instead of stdout
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Agent corpus: repos + lists + unlisted (no raw GitHub payloads)
        #[arg(long)]
        for_agent: bool,
    },
    /// Import or inspect a classification proposal
    #[command(subcommand)]
    Propose(ProposeCommand),
    /// Show the current or named proposal as an apply plan
    Plan {
        #[command(subcommand)]
        command: PlanCommand,
    },
    /// Apply a proposal to GitHub (dry-run unless --confirm)
    Apply {
        /// Proposal / plan id
        #[arg(long)]
        plan: String,
        /// Perform the GitHub mutations
        #[arg(long)]
        confirm: bool,
    },
    /// Raw GraphQL escape hatch (reads by default)
    Request {
        /// GraphQL query or mutation document
        #[arg(long)]
        query: String,
        /// JSON object of GraphQL variables
        #[arg(long)]
        vars: Option<String>,
        /// Allow a mutation (writes GitHub)
        #[arg(long)]
        write: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum ListsCommand {
    /// List local Star Lists
    List {
        /// Include tombstoned lists
        #[arg(long)]
        all: bool,
    },
    /// Resolve a name, slug, or GitHub id to a local list
    Resolve {
        #[arg(long)]
        name: String,
    },
    /// Show one list and its repositories
    Show {
        id: String,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Create a local draft list (GitHub create happens on apply)
    Create {
        name: String,
        #[arg(long)]
        desc: Option<String>,
        #[arg(long)]
        slug: Option<String>,
        /// Create as public on GitHub when applied
        #[arg(long)]
        public: bool,
    },
    /// Rename a local list (marks synced lists dirty)
    Rename { id: String, name: String },
    /// Update list description
    Update {
        id: String,
        #[arg(long)]
        desc: Option<String>,
        #[arg(long)]
        public: Option<bool>,
    },
    /// Mark a list for deletion (drafts with no GitHub id are removed)
    Delete { id: String },
}

#[derive(Subcommand, Debug)]
pub enum ReposCommand {
    /// List starred repositories
    List {
        /// Only repos in this list (slug/name/id)
        #[arg(long)]
        list: Option<String>,
        /// Only repos in no list
        #[arg(long, conflicts_with = "list")]
        unlisted: bool,
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Search name, description, or topics
        #[arg(long, short)]
        query: Option<String>,
    },
    /// Show one repository and its lists
    Show { id: String },
}

#[derive(Subcommand, Debug)]
pub enum ProposeCommand {
    /// Import a proposal JSON (does not write GitHub)
    Import {
        path: PathBuf,
        /// Replace an existing draft proposal with the same notes/id
        #[arg(long)]
        replace: bool,
    },
    /// Show the last imported proposal JSON
    Show { id: Option<String> },
}

#[derive(Subcommand, Debug)]
pub enum PlanCommand {
    /// Expand a proposal into concrete mutations
    Show { id: Option<String> },
}
