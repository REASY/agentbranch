use crate::cli::ShellArgs;
use crate::error::AppError;
use crate::policy::secrets::merge_env_inputs;
use crate::session::exec::{
    SessionSshRequest, build_session_ssh_args, ensure_instance_running, materialize_guest_secret,
    resolve_connection, run_host_command,
};
use crate::types::SessionName;

pub fn run(args: ShellArgs) -> Result<(), AppError> {
    let session_name_raw = args.session.session.resolve_owned()?;
    let session_name = SessionName::try_from(session_name_raw.as_str())?;
    let connection = resolve_connection(&session_name)?;
    if args.session.json {
        println!(
            "{}",
            serde_json::json!({
                "ssh_config_file": connection.ssh_config_file,
                "host_alias": connection.host_alias,
                "workdir": connection.guest_repo_path,
            })
        );
        return Ok(());
    }
    ensure_instance_running(&connection)?;

    let env = merge_env_inputs(&args.env.env, &args.env.env_file)?;
    let guest_secret = materialize_guest_secret(&connection, &env)?;
    let ssh_args = build_session_ssh_args(
        &connection,
        SessionSshRequest {
            forward_agent: args.forward_ssh_agent,
            force_tty: true,
            guest_secret_file: guest_secret.as_ref().map(|path| path.as_path()),
            command: None,
        },
    );
    run_host_command("ssh", &ssh_args)
}
