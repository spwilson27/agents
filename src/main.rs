use std::path::PathBuf;
use std::process;

use agents::{AgentCli, BugBashPhase, Phase, PipeCleanPhase};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "agents",
    about = "Manage AI agent instruction files.",
    arg_required_else_help = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Copy .agents/AGENT.md to all agent CLI instruction files.")]
    Doc {
        #[arg(long, default_value = ".", help = "Project root directory.")]
        root: PathBuf,
    },
    #[command(
        about = "Generate a commit message from the staged diff with an agent CLI, then open it in $EDITOR."
    )]
    Commit {
        #[arg(
            long,
            default_value = "codex",
            value_enum,
            help = "Agent CLI to use for generating the initial commit message."
        )]
        cli: AgentCli,
        #[arg(long, default_value = ".", help = "Git repository root directory.")]
        root: PathBuf,
    },
    #[command(
        about = "Run the three-phase todo-workflow orchestration (plan, implement, land)."
    )]
    TodoWorkflow {
        #[arg(
            long,
            default_value = "claude",
            value_enum,
            help = "Agent CLI to drive the orchestration."
        )]
        cli: AgentCli,
        #[arg(long, default_value = ".", help = "Repository root directory.")]
        root: PathBuf,
        #[arg(
            long,
            value_enum,
            default_value = "all",
            help = "Which phase(s) to run."
        )]
        phase: Phase,
        #[arg(long, help = "Print the resolved plan and exit without invoking the agent.")]
        dry_run: bool,
    },
    #[command(
        about = "Run the two-phase pipeclean orchestration (fix local + CI, then review)."
    )]
    PipeClean {
        #[arg(
            long,
            default_value = "claude",
            value_enum,
            help = "Agent CLI to drive the orchestration."
        )]
        cli: AgentCli,
        #[arg(long, default_value = ".", help = "Repository root directory.")]
        root: PathBuf,
        #[arg(
            long,
            value_enum,
            default_value = "all",
            help = "Which phase(s) to run."
        )]
        phase: PipeCleanPhase,
        #[arg(long, help = "Print the resolved plan and exit without invoking the agent.")]
        dry_run: bool,
    },
    #[command(
        about = "Run the final-review workflow (bookkeeping, rebase, presubmit, PR, two review passes)."
    )]
    FinalReview {
        #[arg(
            long,
            default_value = "claude",
            value_enum,
            help = "Agent CLI to drive the orchestration."
        )]
        cli: AgentCli,
        #[arg(long, default_value = ".", help = "Repository root directory.")]
        root: PathBuf,
        #[arg(long, help = "Print the resolved plan and exit without invoking the agent.")]
        dry_run: bool,
    },
    #[command(
        about = "Run the four-phase bug-bash orchestration (hunt, reproduce, fix, land)."
    )]
    BugBash {
        #[arg(
            long,
            default_value = "claude",
            value_enum,
            help = "Agent CLI to drive the orchestration."
        )]
        cli: AgentCli,
        #[arg(long, default_value = ".", help = "Repository root directory.")]
        root: PathBuf,
        #[arg(
            long,
            value_enum,
            default_value = "all",
            help = "Which phase(s) to run."
        )]
        phase: BugBashPhase,
        #[arg(long, help = "Print the resolved plan and exit without invoking the agent.")]
        dry_run: bool,
    },
    #[command(about = "Save a file as a prompt for later use.")]
    SavePrompt {
        #[arg(help = "Path to the file to save as a prompt.")]
        file: PathBuf,
        #[arg(long, help = "Name for the prompt (defaults to filename without extension).")]
        name: Option<String>,
        #[arg(long, help = "Overwrite existing prompt without confirmation.")]
        force: bool,
    },
    #[command(about = "Print a saved prompt or list available prompts.")]
    Prompt {
        #[arg(help = "Name of the prompt to print.")]
        name: Option<String>,
        #[arg(long, help = "List all available prompts.")]
        list: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Command::Doc { root }) => match agents::doc(&root) {
            Ok(paths) => {
                for path in paths {
                    let rel = path.strip_prefix(&root).unwrap_or(&path);
                    println!("  {}", rel.display());
                }
            }
            Err(err) => {
                eprintln!("{err}");
                process::exit(1);
            }
        },
        Some(Command::Commit { cli, root }) => {
            if let Err(err) = agents::commit(&root, cli) {
                eprintln!("{err}");
                process::exit(1);
            }
        }
        Some(Command::TodoWorkflow {
            cli,
            root,
            phase,
            dry_run,
        }) => {
            if let Err(err) = agents::todo_workflow(&root, cli, phase, dry_run) {
                eprintln!("{err}");
                process::exit(1);
            }
        }
        Some(Command::PipeClean {
            cli,
            root,
            phase,
            dry_run,
        }) => {
            if let Err(err) = agents::pipeclean(&root, cli, phase, dry_run) {
                eprintln!("{err}");
                process::exit(1);
            }
        }
        Some(Command::FinalReview { cli, root, dry_run }) => {
            if let Err(err) = agents::final_review(&root, cli, dry_run) {
                eprintln!("{err}");
                process::exit(1);
            }
        }
        Some(Command::BugBash {
            cli,
            root,
            phase,
            dry_run,
        }) => {
            if let Err(err) = agents::bug_bash(&root, cli, phase, dry_run) {
                eprintln!("{err}");
                process::exit(1);
            }
        }
        Some(Command::SavePrompt { file, name, force }) => {
            if let Err(err) = agents::save_prompt(&file, name.as_deref(), force) {
                eprintln!("{err}");
                process::exit(1);
            }
        }
        Some(Command::Prompt { name, list }) => {
            if let Err(err) = agents::prompt(name.as_deref(), list) {
                eprintln!("{err}");
                process::exit(1);
            }
        }
        None => unreachable!("clap exits after printing help when no subcommand is given"),
    }
}
