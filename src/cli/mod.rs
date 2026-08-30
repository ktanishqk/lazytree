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
        /// Disable canonical mount-namespace paths (path-stable caches)
        #[arg(long)]
        no_canonical: bool,
        /// Command and args (use `--` to disambiguate)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
    /// Manage shared semantic build caches
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
    /// Cursor IDE integration helpers
    Cursor {
        #[command(subcommand)]
        command: CursorCommands,
    },
}

#[derive(Debug, Subcommand)]
pub enum CacheCommands {
    /// Copy this session's target dir into the repo shared seed
    Promote { session: String },
    /// Populate session target from the shared seed
    Seed { session: String },
}

#[derive(Debug, Subcommand)]
pub enum CursorCommands {
    /// Open session root in Cursor/VS Code so UI Git binds to our branch
    Open {
        session: String,
        /// Editor binary (default: cursor, then code, or $LAZYTREE_EDITOR)
        #[arg(long)]
        editor: Option<String>,
        /// Print JSON path/branch and do not launch an editor
        #[arg(long)]
        json: bool,
        /// Print path/branch only; do not launch an editor
        #[arg(long)]
        dry_run: bool,
    },
    /// Print hook/skill setup instructions (and optionally install into a project)
    Setup {
        /// Copy `.cursor` hooks + skill into this project directory
        #[arg(long)]
        target: Option<PathBuf>,
        #[arg(long)]
        json: bool,
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
        Commands::Exec {
            session,
            no_canonical,
            command,
        } => {
            let opts = crate::runtime::ExecOptions {
                canonical: !no_canonical,
            };
            let code = sessions.exec_with(&session, &command, &opts)?;
            return Ok(ExitCode::from(code as u8));
        }
        Commands::Cache { command } => match command {
            CacheCommands::Promote { session } => {
                sessions.cache_promote(&session)?;
                println!("Promoted target cache from {session} to shared seed");
            }
            CacheCommands::Seed { session } => {
                if sessions.cache_seed(&session)? {
                    println!("Seeded target cache for {session} from shared seed");
                } else {
                    println!("No shared target seed available for {session}");
                }
            }
        },
        Commands::Cursor { command } => match command {
            CursorCommands::Open {
                session,
                editor,
                json,
                dry_run,
            } => {
                let s = sessions.get(&session)?;
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&crate::cursor_integration::open_session_info(
                            &s
                        ))?
                    );
                } else if dry_run {
                    println!("{}\t{}", s.root_path().display(), s.branch);
                } else {
                    let ed = editor.unwrap_or_else(crate::cursor_integration::resolve_editor);
                    println!(
                        "Opening {} (branch {}) in {ed}",
                        s.root_path().display(),
                        s.branch
                    );
                    crate::cursor_integration::open_session(&s, &ed)?;
                }
            }
            CursorCommands::Setup { target, json } => {
                let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                if let Some(target) = target {
                    let bundled = crate::cursor_integration::find_bundled_assets()?;
                    let written = crate::cursor_integration::install_into_project(&bundled, &target)?;
                    if json {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "target": target,
                                "bundled_from": bundled,
                                "written": written,
                            }))?
                        );
                    } else {
                        println!(
                            "Installed LazyTree Cursor assets into {}",
                            target.join(".cursor").display()
                        );
                        for p in written {
                            println!("  {}", p.display());
                        }
                    }
                } else {
                    crate::cursor_integration::print_integration_hints(&cwd);
                }
            }
        },
    }

    Ok(ExitCode::SUCCESS)
}
