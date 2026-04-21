use std::path::PathBuf;
use std::process;

use agents::{AgentCli, Phase};
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
        #[arg(
            long,
            help = "Directory containing prompt_0{1,2,3}.md; overrides AGENTS_PROMPTS_DIR."
        )]
        prompts_dir: Option<PathBuf>,
        #[arg(long, help = "Print the resolved plan and exit without invoking the agent.")]
        dry_run: bool,
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
            prompts_dir,
            dry_run,
        }) => {
            if let Err(err) = agents::todo_workflow(
                &root,
                cli,
                phase,
                prompts_dir.as_deref(),
                dry_run,
            ) {
                eprintln!("{err}");
                process::exit(1);
            }
        }
        None => unreachable!("clap exits after printing help when no subcommand is given"),
    }
}
