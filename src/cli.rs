use crate::error::ValidationError;
use crate::ports::PublishedPort;
use crate::types::{DiskSize, MemorySize};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(
    name = "agbranch",
    version,
    about = "Disposable coding sessions for AI agents, synced back through git",
    long_about = "Create isolated Lima VMs for sandbox or git-native coding sessions, run an agent inside the guest, and explicitly export or sync the result back to the host.",
    after_help = "Quick start:
  agbranch doctor
  agbranch base prepare
  agbranch launch --session demo --seed . --agent codex

Run `agbranch <command> --help` for command-specific examples."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Build or inspect the reusable prepared VM.
    Base(BaseArgs),
    /// Create a disposable sandbox session.
    Launch(LaunchArgs),
    /// Create a git-native repository session.
    Open(OpenArgs),
    /// Copy an artifact from a sandbox guest to the host.
    Export(ExportArgs),
    /// Attach to a session-owned shell or agent.
    Attach(AttachArgs),
    /// Start or stop a session-owned coding agent.
    Agent(AgentArgs),
    /// Inspect or change remembered credential-import policies.
    Auth(AuthArgs),
    /// Stop an agent, optionally forcing the VM to stop.
    Kill(KillArgs),
    /// List active sessions, or include session history.
    Ps(ListArgs),
    /// Show detailed state for one session.
    Show(SessionArgs),
    /// Show published localhost ports and live listeners.
    Ports(PortsArgs),
    /// Start a stopped session VM.
    Start(SessionArgs),
    /// Stop a session VM without closing it.
    Stop(SessionArgs),
    /// Open a fresh tmux shell in the guest.
    Shell(ShellArgs),
    /// Open a raw SSH connection to the guest.
    Ssh(SshArgs),
    /// Run a non-interactive command in the guest.
    Run(RunArgs),
    /// Sync guest git commits to the host review branch.
    SyncBack(SyncBackArgs),
    /// Destroy a session after syncing or discarding it.
    Close(CloseArgs),
    /// Reclaim stale local state and obsolete caches.
    Gc(JsonFlag),
    /// Read session, provision, sync, guest, or kernel logs.
    Logs(LogsArgs),
    /// Stream session snapshots and events.
    Watch(WatchArgs),
    /// Recover a session stuck in a transitional state.
    Repair(SessionArgs),
    /// Resume a launch from its last completed phase.
    Retry(RetryArgs),
    /// Generate a shell completion script on stdout.
    Completions(CompletionsArgs),
    /// Check host prerequisites and local state.
    Doctor(JsonFlag),
    /// Internal / troubleshooting subcommands. Hidden from top-level help
    /// but visible via `agbranch internal --help`.
    #[command(hide = true)]
    Internal(InternalArgs),
}

#[derive(Debug, Args)]
pub struct InternalArgs {
    #[command(subcommand)]
    pub action: InternalAction,
}

#[derive(Debug, Subcommand)]
pub enum InternalAction {
    /// Force extraction of the embedded Lima asset bundle into the
    /// state-root cache. Used by the standalone-binary E2E test and for
    /// troubleshooting cold-cache state.
    ExtractAssets(ExtractAssetsArgs),
}

#[derive(Debug, Args)]
pub struct ExtractAssetsArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct BaseArgs {
    #[command(subcommand)]
    pub action: BaseAction,
}

#[derive(Debug, Subcommand)]
pub enum BaseAction {
    Prepare(PrepareArgs),
    Show(BaseShowArgs),
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
pub struct BaseShowArgs {
    #[arg(long)]
    pub json: bool,
    #[arg(long)]
    pub require_ready: bool,
}

#[derive(Debug, Args)]
#[command(after_help = "Examples:
  agbranch launch --session demo --seed .
  agbranch launch --session web --publish 3000 --agent codex --auth import

Post-clone failures can be resumed with `agbranch retry SESSION`.")]
pub struct LaunchArgs {
    /// Unique session name (letters, digits, dots, underscores, and hyphens).
    #[arg(long)]
    pub session: String,
    /// Host file or directory copied into the guest workspace.
    #[arg(long)]
    pub seed: Option<PathBuf>,
    /// Coding agent to start inside the guest.
    #[arg(long, value_parser = ["codex", "claude", "gemini"])]
    pub agent: Option<String>,
    /// Attach after launch (agent when present, otherwise shell).
    #[arg(long, conflicts_with_all = ["detach", "json"])]
    pub attach: bool,
    /// Return after launch without attaching.
    #[arg(long, conflicts_with = "attach")]
    pub detach: bool,
    /// Control host auth import. Omit to reuse the provider's remembered choice,
    /// prompting once when no choice exists.
    #[arg(long, value_enum, requires = "agent")]
    pub auth: Option<AuthMode>,
    /// Publish a guest port on host localhost. Accepts GUEST_PORT or
    /// HOST_PORT:GUEST_PORT, with an optional /tcp or /udp suffix.
    #[arg(long = "publish", value_name = "PORT")]
    pub publish: Vec<PublishedPort>,
    /// Number of virtual CPUs for this session.
    #[arg(long)]
    pub cpus: Option<u16>,
    /// Guest memory size, for example 8GiB.
    #[arg(long)]
    pub memory: Option<MemorySize>,
    /// Guest disk size, for example 100GiB.
    #[arg(long)]
    pub disk: Option<DiskSize>,
    /// Emit a machine-readable result and do not attach.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(after_help = "Examples:
  agbranch open --session feature-x --repo .
  agbranch open --session api --repo . --base origin/main --publish 8080 --agent claude

Changes sync to the host review branch `agbranch/SESSION`; the checked-out host branch is never rewritten.")]
pub struct OpenArgs {
    /// Unique session name (letters, digits, dots, underscores, and hyphens).
    #[arg(long)]
    pub session: String,
    /// Host git worktree to seed into the guest.
    #[arg(long)]
    pub repo: PathBuf,
    /// Git ref used as the session baseline (defaults to the current branch).
    #[arg(long)]
    pub base: Option<String>,
    /// Coding agent to start inside the guest.
    #[arg(long, value_parser = ["codex", "claude", "gemini"])]
    pub agent: Option<String>,
    /// Attach after open (agent when present, otherwise shell).
    #[arg(long, conflicts_with_all = ["detach", "json"])]
    pub attach: bool,
    /// Return after open without attaching.
    #[arg(long, conflicts_with = "attach")]
    pub detach: bool,
    /// Control host auth import. Omit to reuse the provider's remembered choice,
    /// prompting once when no choice exists.
    #[arg(long, value_enum, requires = "agent")]
    pub auth: Option<AuthMode>,
    /// Publish a guest port on host localhost. Accepts GUEST_PORT or
    /// HOST_PORT:GUEST_PORT, with an optional /tcp or /udp suffix.
    #[arg(long = "publish", value_name = "PORT")]
    pub publish: Vec<PublishedPort>,
    /// Number of virtual CPUs for this session.
    #[arg(long)]
    pub cpus: Option<u16>,
    /// Guest memory size, for example 8GiB.
    #[arg(long)]
    pub memory: Option<MemorySize>,
    /// Guest disk size, for example 100GiB.
    #[arg(long)]
    pub disk: Option<DiskSize>,
    /// Emit a machine-readable result and do not attach.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct RetryArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
#[command(after_help = "Install examples:
  agbranch completions bash > ~/.local/share/bash-completion/completions/agbranch
  agbranch completions zsh > ~/.zfunc/_agbranch
  agbranch completions fish > ~/.config/fish/completions/agbranch.fish")]
pub struct CompletionsArgs {
    #[arg(value_enum)]
    pub shell: CompletionShell,
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
pub struct AuthArgs {
    #[command(subcommand)]
    pub action: AuthAction,
}

#[derive(Debug, Subcommand)]
pub enum AuthAction {
    /// Show the remembered policy for every supported provider.
    List(JsonFlag),
    /// Set a provider's non-interactive credential-import policy.
    Set(AuthSetArgs),
    /// Forget one provider's policy, or all remembered policies.
    Reset(AuthResetArgs),
}

#[derive(Debug, Args)]
pub struct AuthSetArgs {
    #[arg(value_enum)]
    pub provider: ProviderArg,
    #[arg(value_enum)]
    pub policy: AuthPreferencePolicy,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AuthResetArgs {
    #[arg(value_enum, required_unless_present = "all", conflicts_with = "all")]
    pub provider: Option<ProviderArg>,
    #[arg(long)]
    pub all: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Args)]
pub struct AgentStartArgs {
    #[command(flatten)]
    pub session: SessionSelector,
    #[arg(long, value_parser = ["codex", "claude", "gemini"])]
    pub provider: String,
    /// Control host auth import. Omit to reuse the provider's remembered choice,
    /// prompting once when no choice exists.
    #[arg(long, value_enum)]
    pub auth: Option<AuthMode>,
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
pub struct PortsArgs {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AuthMode {
    Import,
    None,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ProviderArg {
    Codex,
    Claude,
    Gemini,
}

impl From<ProviderArg> for crate::types::ProviderKind {
    fn from(value: ProviderArg) -> Self {
        match value {
            ProviderArg::Codex => Self::Codex,
            ProviderArg::Claude => Self::Claude,
            ProviderArg::Gemini => Self::Gemini,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AuthPreferencePolicy {
    Import,
    None,
}

impl AuthPreferencePolicy {
    pub fn import(self) -> bool {
        matches!(self, Self::Import)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Elvish,
    #[value(name = "powershell")]
    PowerShell,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_defaults_timeout_to_twenty_minutes() {
        let cli = Cli::parse_from(["agbranch", "base", "prepare"]);

        let Command::Base(args) = cli.command else {
            panic!("expected base command");
        };
        let BaseAction::Prepare(args) = args.action else {
            panic!("expected base prepare command");
        };

        assert_eq!(args.timeout, Duration::from_secs(20 * 60));
    }

    #[test]
    fn prepare_accepts_explicit_timeout_override() {
        let cli = Cli::parse_from(["agbranch", "base", "prepare", "--timeout", "35m"]);

        let Command::Base(args) = cli.command else {
            panic!("expected base command");
        };
        let BaseAction::Prepare(args) = args.action else {
            panic!("expected base prepare command");
        };

        assert_eq!(args.timeout, Duration::from_secs(35 * 60));
    }

    #[test]
    fn base_prepare_accepts_existing_prepare_flags() {
        let cli = Cli::parse_from(["agbranch", "base", "prepare", "--timeout", "35m", "--json"]);

        let Command::Base(args) = cli.command else {
            panic!("expected base command");
        };
        let BaseAction::Prepare(args) = args.action else {
            panic!("expected base prepare command");
        };

        assert_eq!(args.timeout, Duration::from_secs(35 * 60));
        assert!(args.json);
    }

    #[test]
    fn base_show_accepts_json_and_require_ready() {
        let cli = Cli::parse_from(["agbranch", "base", "show", "--json", "--require-ready"]);

        let Command::Base(args) = cli.command else {
            panic!("expected base command");
        };
        let BaseAction::Show(args) = args.action else {
            panic!("expected base show command");
        };

        assert!(args.json);
        assert!(args.require_ready);
    }

    #[test]
    fn top_level_prepare_is_not_accepted() {
        let err = Cli::try_parse_from(["agbranch", "prepare"]).expect_err("prepare is removed");

        assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn launch_accepts_explicit_auth_modes() {
        for (raw, expected) in [
            ("import", AuthMode::Import),
            ("none", AuthMode::None),
            ("ask", AuthMode::Ask),
        ] {
            let cli = Cli::parse_from([
                "agbranch",
                "launch",
                "--session",
                "demo",
                "--agent",
                "codex",
                "--auth",
                raw,
            ]);
            let Command::Launch(args) = cli.command else {
                panic!("expected launch command");
            };
            assert_eq!(args.auth, Some(expected));
        }
    }

    #[test]
    fn launch_auth_requires_an_agent() {
        let err =
            Cli::try_parse_from(["agbranch", "launch", "--session", "demo", "--auth", "none"])
                .expect_err("auth without an agent should be rejected");

        assert_eq!(err.kind(), clap::error::ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn launch_and_open_accept_repeatable_port_publications() {
        let cli = Cli::parse_from([
            "agbranch",
            "launch",
            "--session",
            "demo",
            "--publish",
            "3000",
            "--publish",
            "8080:80/udp",
        ]);
        let Command::Launch(args) = cli.command else {
            panic!("expected launch");
        };
        assert_eq!(args.publish.len(), 2);
        assert_eq!(args.publish[0].host_port, 3000);
        assert_eq!(args.publish[1].guest_port, 80);

        let cli = Cli::parse_from([
            "agbranch",
            "open",
            "--session",
            "demo",
            "--repo",
            ".",
            "--publish",
            "5173",
        ]);
        let Command::Open(args) = cli.command else {
            panic!("expected open");
        };
        assert_eq!(args.publish[0].host_port, 5173);
    }

    #[test]
    fn launch_and_open_accept_explicit_attachment_modes() {
        let cli = Cli::parse_from(["agbranch", "launch", "--session", "demo", "--detach"]);
        let Command::Launch(args) = cli.command else {
            panic!("expected launch");
        };
        assert!(args.detach);
        assert!(!args.attach);

        let cli = Cli::parse_from([
            "agbranch",
            "open",
            "--session",
            "demo",
            "--repo",
            ".",
            "--attach",
        ]);
        let Command::Open(args) = cli.command else {
            panic!("expected open");
        };
        assert!(args.attach);
        assert!(!args.detach);
    }

    #[test]
    fn attachment_modes_reject_conflicting_output_or_direction() {
        let conflict = Cli::try_parse_from([
            "agbranch",
            "launch",
            "--session",
            "demo",
            "--attach",
            "--detach",
        ])
        .expect_err("attachment direction must be unique");
        assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);

        let json_conflict = Cli::try_parse_from([
            "agbranch",
            "open",
            "--session",
            "demo",
            "--repo",
            ".",
            "--attach",
            "--json",
        ])
        .expect_err("interactive attach conflicts with json");
        assert_eq!(
            json_conflict.kind(),
            clap::error::ErrorKind::ArgumentConflict
        );
    }

    #[test]
    fn ports_accepts_positional_session_and_json() {
        let cli = Cli::parse_from(["agbranch", "ports", "demo", "--json"]);
        let Command::Ports(args) = cli.command else {
            panic!("expected ports");
        };
        assert_eq!(args.session.resolve().expect("session"), "demo");
        assert!(args.json);
    }

    #[test]
    fn retry_accepts_positional_or_flag_session() {
        let cli = Cli::parse_from(["agbranch", "retry", "demo", "--json"]);
        let Command::Retry(args) = cli.command else {
            panic!("expected retry");
        };
        assert_eq!(args.session.resolve().expect("session"), "demo");
        assert!(args.json);

        let cli = Cli::parse_from(["agbranch", "retry", "--session", "flagged"]);
        let Command::Retry(args) = cli.command else {
            panic!("expected retry");
        };
        assert_eq!(args.session.resolve().expect("session"), "flagged");
    }

    #[test]
    fn open_and_agent_start_accept_auth_policy() {
        let cli = Cli::parse_from([
            "agbranch",
            "open",
            "--session",
            "demo",
            "--repo",
            ".",
            "--agent",
            "claude",
            "--auth",
            "none",
        ]);
        let Command::Open(args) = cli.command else {
            panic!("expected open command");
        };
        assert_eq!(args.auth, Some(AuthMode::None));

        let cli = Cli::parse_from([
            "agbranch",
            "agent",
            "start",
            "demo",
            "--provider",
            "gemini",
            "--auth",
            "ask",
        ]);
        let Command::Agent(args) = cli.command else {
            panic!("expected agent command");
        };
        let AgentAction::Start(args) = args.action else {
            panic!("expected agent start command");
        };
        assert_eq!(args.auth, Some(AuthMode::Ask));
    }

    #[test]
    fn auth_management_parses_list_set_and_reset() {
        let cli = Cli::parse_from(["agbranch", "auth", "list", "--json"]);
        let Command::Auth(args) = cli.command else {
            panic!("expected auth");
        };
        let AuthAction::List(args) = args.action else {
            panic!("expected list");
        };
        assert!(args.json);

        let cli = Cli::parse_from(["agbranch", "auth", "set", "codex", "import"]);
        let Command::Auth(args) = cli.command else {
            panic!("expected auth");
        };
        let AuthAction::Set(args) = args.action else {
            panic!("expected set");
        };
        assert_eq!(args.provider, ProviderArg::Codex);
        assert_eq!(args.policy, AuthPreferencePolicy::Import);

        let cli = Cli::parse_from(["agbranch", "auth", "reset", "--all"]);
        let Command::Auth(args) = cli.command else {
            panic!("expected auth");
        };
        let AuthAction::Reset(args) = args.action else {
            panic!("expected reset");
        };
        assert!(args.all);
        assert_eq!(args.provider, None);
    }

    #[test]
    fn auth_reset_requires_exactly_one_scope() {
        let missing = Cli::try_parse_from(["agbranch", "auth", "reset"])
            .expect_err("reset needs a provider or --all");
        assert_eq!(
            missing.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );

        let conflict = Cli::try_parse_from(["agbranch", "auth", "reset", "codex", "--all"])
            .expect_err("provider conflicts with --all");
        assert_eq!(conflict.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn completions_accepts_every_supported_shell() {
        for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
            let cli = Cli::parse_from(["agbranch", "completions", shell]);
            let Command::Completions(args) = cli.command else {
                panic!("expected completions for {shell}");
            };
            assert_eq!(
                args.shell,
                match shell {
                    "bash" => CompletionShell::Bash,
                    "zsh" => CompletionShell::Zsh,
                    "fish" => CompletionShell::Fish,
                    "elvish" => CompletionShell::Elvish,
                    "powershell" => CompletionShell::PowerShell,
                    _ => unreachable!(),
                }
            );
        }
    }
}
