use clap::Parser;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = agbranch::cli::Cli::parse();
    match agbranch::app::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(err.exit_code() as u8)
        }
    }
}
