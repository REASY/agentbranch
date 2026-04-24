use crate::cli::{Cli, Command};
use crate::commands::{
    agent, attach, close, doctor, export, gc, kill, launch, logs, open, prepare, ps, repair, run,
    shell, show, ssh, start, stop, sync_back, watch,
};
use crate::error::AppError;

pub fn run(cli: Cli) -> Result<(), AppError> {
    match cli.command {
        Command::Prepare(args) => prepare::run(args),
        Command::Launch(args) => launch::run(args),
        Command::Open(args) => open::run(args),
        Command::Export(args) => export::run(args),
        Command::Attach(args) => attach::run(args),
        Command::Agent(args) => agent::run(args),
        Command::Kill(args) => kill::run(args),
        Command::Ps(args) => ps::run(args),
        Command::Show(args) => show::run(args),
        Command::Start(args) => start::run(args),
        Command::Stop(args) => stop::run(args),
        Command::Shell(args) => shell::run(args),
        Command::Ssh(args) => ssh::run(args),
        Command::Run(args) => run::run(args),
        Command::SyncBack(args) => sync_back::run(args),
        Command::Close(args) => close::run(args),
        Command::Gc(args) => gc::run(args),
        Command::Logs(args) => logs::run(args),
        Command::Watch(args) => watch::run(args),
        Command::Repair(args) => repair::run(args),
        Command::Doctor(args) => doctor::run(args),
    }
}
