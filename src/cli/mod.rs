use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::doctor;
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
        /// Base revision (commit/branch) for the session branch
        #[arg(long = "from")]
        from: Option<String>,
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
    /// Show session status
    Status {
        session: String,
        #[arg(long)]
        json: bool,
    },
    /// Show session diff (worktree + staged)
    Diff {
        session: String,
    },
    /// Publish branch to source repo and drop ephemeral workspace state
    Archive {
        session: String,
        #[arg(long)]
        json: bool,
    },
    /// Check metadata vs reality
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Destroy a session
    Destroy {
        session: String,
        /// Skip dirty / unexported-commit checks
        #[arg(long)]
        force: bool,
    },
    /// Run a command in the session root with semantic cache env set
    Exec {
        session: String,
        /// Command and args (use `--` to disambiguate)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
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

pub fn run(cli: Cli) -> Result<ExitCode> {
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
        Commands::Create {
            name,
            repo,
            from,
            json,
        } => {
            let session = sessions.create(&name, repo.as_deref(), from.as_deref())?;
            if json {
                let timings = crate::session::take_last_create_timings();
                let out = serde_json::json!({
                    "id": session.id,
                    "name": session.name,
                    "path": session.root_path(),
                    "repository_id": session.repository_id,
                    "branch": session.branch,
                    "filesystem": session.filesystem,
                    "git": session.git,
                    "semantic": session.semantic,
                    "runtime": session.runtime,
                    "timings": timings,
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
                    let state = if s.lifecycle == "archived" {
                        "archived"
                    } else {
                        s.filesystem.state.as_str()
                    };
                    println!(
                        "{}\t{}\t{}\t{}",
                        s.name,
                        s.id,
                        state,
                        s.root_path().display()
                    );
                }
            }
        }
        Commands::Path { session } => {
            let s = sessions.get(&session)?;
            println!("{}", s.root_path().display());
        }
        Commands::Status { session, json } => {
            let st = sessions.status(&session)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&st)?);
            } else {
                println!("name:                 {}", st.name);
                println!("branch:               {}", st.branch);
                println!("dirty:                {}", st.dirty);
                println!("unexported_commits:   {}", st.unexported_commits);
                println!("filesystem:           {} ({:?})", st.filesystem_state, st.filesystem_backend);
                println!("git:                  {}", st.git_state);
                println!("semantic:             {}", st.semantic_state);
                println!("runtime:              {}", st.runtime_state);
                println!("lifecycle:            {}", st.lifecycle);
                println!("upper_files:          {}", st.upper_files);
                println!("filesystem_bytes:     {}", st.filesystem_bytes_written);
                println!("shared_cache:         {}", st.shared_cache);
                println!("session_cache:        {}", st.session_cache);
                println!("root:                 {}", st.root);
                println!("age_seconds:          {}", st.age_seconds);
            }
        }
        Commands::Diff { session } => {
            let text = sessions.diff(&session)?;
            print!("{text}");
        }
        Commands::Archive { session, json } => {
            let s = sessions.archive(&session)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&s)?);
            } else {
                println!("Archived {}: branch {} published", s.name, s.branch);
            }
        }
        Commands::Doctor { json } => {
            let report = doctor::run_doctor(&paths, &sessions, &repos)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else if report.issues.is_empty() {
                println!("ok: no issues found");
            } else {
                for i in &report.issues {
                    println!("[{}] {}: {}", i.severity, i.code, i.message);
                }
            }
            if !report.ok {
                // Exit 1 without aborting via process::exit so Drop runs cleanly.
                return Ok(ExitCode::from(1));
            }
        }
        Commands::Destroy { session, force } => {
            sessions.destroy(&session, force)?;
            println!("Destroyed session: {session}");
        }
        Commands::Exec { session, command } => {
            let code = sessions.exec(&session, &command)?;
            return Ok(ExitCode::from(code as u8));
        }
    }

    Ok(ExitCode::SUCCESS)
}
