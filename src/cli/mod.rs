use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::metadata::Paths;
use crate::repository::RepositoryStore;
use crate::session::SessionStore;

#[derive(Debug, Parser)]
#[command(name = "lazytree", version, about = "COW development workspaces for parallel coding agents")]
pub struct Cli {
    /// Override LazyTree home directory (default: ~/.lazytree or $LAZYTREE_HOME)
    #[arg(long, global = true)]
    pub home: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Manage registered repositories
    Repo {
        #[command(subcommand)]
        command: RepoCommands,
    },
    /// Create a new COW workspace session
    Create {
        /// Human-readable session name
        name: String,
        /// Repository id or source path (defaults to the only registered repo)
        #[arg(long)]
        repo: Option<String>,
        /// Emit JSON
        #[arg(long)]
        json: bool,
    },
    /// List sessions
    List {
        #[arg(long)]
        json: bool,
    },
    /// Print the merged root path for a session
    Path {
        session: String,
    },
    /// Destroy a session
    Destroy {
        session: String,
        /// Skip dirty checks (M1: always allowed; enforced properly in M3)
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum RepoCommands {
    /// Register a clean Git repository
    Add {
        path: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// List registered repositories
    List {
        #[arg(long)]
        json: bool,
    },
    /// Remove a registered repository (fails if sessions exist)
    Remove {
        repo: String,
        #[arg(long)]
        force: bool,
    },
}

pub fn run(cli: Cli) -> Result<()> {
    let paths = Paths::resolve(cli.home)?;
    paths.ensure_layout()?;

    let repos = RepositoryStore::new(paths.clone());
    let sessions = SessionStore::new(paths.clone(), repos.clone());

    match cli.command {
        Commands::Repo { command } => match command {
            RepoCommands::Add { path, json } => {
                let repo = repos.add(&path)?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&repo)?);
                } else {
                    println!("Repository registered: {}", repo.id);
                    println!("Base: {}", repo.base_path);
                }
            }
            RepoCommands::List { json } => {
                let list = repos.list()?;
                if json {
                    println!("{}", serde_json::to_string_pretty(&list)?);
                } else if list.is_empty() {
                    println!("(no repositories)");
                } else {
                    for r in list {
                        println!("{}\t{}\t{}", r.id, r.state, r.source_path);
                    }
                }
            }
            RepoCommands::Remove { repo, force } => {
                repos.remove(&repo, force)?;
                println!("Removed repository: {repo}");
            }
        },
        Commands::Create { name, repo, json } => {
            let session = sessions.create(&name, repo.as_deref())?;
            if json {
                let out = serde_json::json!({
                    "id": session.id,
                    "name": session.name,
                    "path": session.root_path(),
                    "repository_id": session.repository_id,
                    "filesystem": session.filesystem,
                });
                println!("{}", serde_json::to_string_pretty(&out)?);
            } else {
                println!("{}", session.root_path().display());
            }
        }
        Commands::List { json } => {
            let list = sessions.list()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else if list.is_empty() {
                println!("(no sessions)");
            } else {
                for s in list {
                    println!(
                        "{}\t{}\t{}\t{}",
                        s.name,
                        s.id,
                        s.filesystem.state,
                        s.root_path().display()
                    );
                }
            }
        }
        Commands::Path { session } => {
            let s = sessions.get(&session)?;
            println!("{}", s.root_path().display());
        }
        Commands::Destroy { session, force } => {
            sessions.destroy(&session, force)?;
            println!("Destroyed session: {session}");
        }
    }

    Ok(())
}
