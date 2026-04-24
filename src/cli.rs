use crate::error::ValidationError;
use crate::types::{DiskSize, MemorySize};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "agbranch",
    version,
    about = "Disposable coding sessions for AI agents, synced back through git"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Prepare(PrepareArgs),
    Launch(LaunchArgs),
    Open(OpenArgs),
    Export(ExportArgs),
    Attach(AttachArgs),
    Agent(AgentArgs),
    Kill(KillArgs),
    Ps(ListArgs),
    Show(SessionArgs),
    Start(SessionArgs),
    Stop(SessionArgs),
    Shell(ShellArgs),
    Ssh(SshArgs),
    Run(RunArgs),
    SyncBack(SyncBackArgs),
    Close(CloseArgs),
    Gc(JsonFlag),
    Logs(LogsArgs),
    Watch(WatchArgs),
    Repair(SessionArgs),
    Doctor(JsonFlag),
}

#[derive(Debug, Args)]
pub struct PrepareArgs {
    #[arg(long)]
    pub rebuild: bool,
    #[arg(long, default_value = "20m", value_parser = humantime::parse_duration)]
    pub timeout: Duration,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LaunchArgs {
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub seed: Option<PathBuf>,
    #[arg(long, value_parser = ["codex", "claude", "gemini"])]
    pub agent: Option<String>,
    #[arg(long)]
    pub cpus: Option<u16>,
    #[arg(long)]
    pub memory: Option<MemorySize>,
    #[arg(long)]
    pub disk: Option<DiskSize>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct OpenArgs {
    #[arg(long)]
    pub session: String,
    #[arg(long)]
    pub repo: PathBuf,
    #[arg(long)]
    pub base: Option<String>,
    #[arg(long, value_parser = ["codex", "claude", "gemini"])]
    pub agent: Option<String>,
    #[arg(long)]
    pub cpus: Option<u16>,
    #[arg(long)]
    pub memory: Option<MemorySize>,
    #[arg(long)]
    pub disk: Option<DiskSize>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long = "from")]
    pub from_guest_path: String,
    #[arg(long = "to")]
    pub to_host_path: PathBuf,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AttachArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long)]
    pub shell: bool,
    #[arg(long)]
    pub agent: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AgentArgs {
    #[command(subcommand)]
    pub action: AgentAction,
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    Start(AgentStartArgs),
    Stop(AgentStopArgs),
}

#[derive(Debug, Args)]
pub struct AgentStartArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long, value_parser = ["codex", "claude", "gemini"])]
    pub provider: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AgentStopArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct KillArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long)]
    pub force: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct ListArgs {
    #[arg(short = 'a', long)]
    pub all: bool,
    #[arg(long)]
    pub search: Option<String>,
    #[arg(long)]
    pub state: Option<String>,
    #[arg(long)]
    pub sort: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SessionSelector {
    #[arg(value_name = "SESSION", required_unless_present = "session_flag")]
    pub positional_session: Option<String>,
    #[arg(long = "session", value_name = "SESSION")]
    pub session_flag: Option<String>,
}

impl SessionSelector {
    pub fn from_session(session: impl Into<String>) -> Self {
        Self {
            positional_session: Some(session.into()),
            session_flag: None,
        }
    }

    pub fn resolve(&self) -> Result<&str, ValidationError> {
        match (
            self.positional_session.as_deref(),
            self.session_flag.as_deref(),
        ) {
            (Some(positional), Some(flag)) if positional != flag => {
                Err(ValidationError::SessionSelectorConflict {
                    positional: positional.to_owned(),
                    flag: flag.to_owned(),
                })
            }
            (Some(positional), _) => Ok(positional),
            (None, Some(flag)) => Ok(flag),
            (None, None) => Err(ValidationError::SessionSelectorRequired),
        }
    }

    pub fn resolve_owned(&self) -> Result<String, ValidationError> {
        self.resolve().map(ToOwned::to_owned)
    }
}

#[derive(Debug, Args)]
pub struct SessionArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct JsonFlag {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct EnvArgs {
    #[arg(long = "env")]
    pub env: Vec<String>,
    #[arg(long = "env-file")]
    pub env_file: Vec<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ShellArgs {
    #[command(flatten)]
    pub session: SessionArgs,
    #[arg(long)]
    pub forward_ssh_agent: bool,
    #[command(flatten)]
    pub env: EnvArgs,
}

#[derive(Debug, Args)]
pub struct SshArgs {
    #[command(flatten)]
    pub session: SessionArgs,
    #[arg(long)]
    pub forward_ssh_agent: bool,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[command(flatten)]
    pub env: EnvArgs,
    #[arg(last = true, required = true)]
    pub command: Vec<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct SyncBackArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long)]
    pub yes: bool,
    #[arg(long = "export-patch")]
    pub export_patch: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct CloseArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long)]
    pub sync: bool,
    #[arg(long)]
    pub discard: bool,
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct LogsArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long, value_enum, default_value_t = LogSource::Events)]
    pub source: LogSource,
    #[arg(long)]
    pub follow: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct WatchArgs {
    #[arg(long)]
    pub session: Option<String>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LogSource {
    Events,
    Provision,
    Sync,
    Guest,
    Kernel,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_defaults_timeout_to_twenty_minutes() {
        let cli = Cli::parse_from(["agbranch", "prepare"]);

        let Command::Prepare(args) = cli.command else {
            panic!("expected prepare command");
        };

        assert_eq!(args.timeout, Duration::from_secs(20 * 60));
    }

    #[test]
    fn prepare_accepts_explicit_timeout_override() {
        let cli = Cli::parse_from(["agbranch", "prepare", "--timeout", "35m"]);

        let Command::Prepare(args) = cli.command else {
            panic!("expected prepare command");
        };

        assert_eq!(args.timeout, Duration::from_secs(35 * 60));
    }
}
