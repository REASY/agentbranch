use crate::cli::RunArgs;
use crate::error::AppError;
use crate::policy::secrets::merge_env_inputs;
use crate::session::exec::{
    SessionSshRequest, build_session_ssh_args, ensure_instance_running, materialize_guest_secret,
    resolve_connection, run_host_command,
};
use crate::types::SessionName;

pub fn run(args: RunArgs) -> Result<(), AppError> {
    let env = merge_env_inputs(&args.env.env, &args.env.env_file)?;
    let session_name_raw = args.session.resolve_owned()?;
    let session_name = SessionName::try_from(session_name_raw.as_str())?;
    let connection = resolve_connection(&session_name)?;
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "ssh_config_file": connection.ssh_config_file,
                "host_alias": connection.host_alias,
                "workdir": connection.guest_repo_path,
                "command": args.command,
            })
        );
        return Ok(());
    }
    ensure_instance_running(&connection)?;
    let guest_secret = materialize_guest_secret(&connection, &env)?;
    let ssh_args = build_session_ssh_args(
        &connection,
        SessionSshRequest {
            forward_agent: false,
            force_tty: false,
            guest_secret_file: guest_secret.as_ref().map(|path| path.as_path()),
            command: Some(&args.command),
        },
    );
    run_host_command("ssh", &ssh_args)
}
