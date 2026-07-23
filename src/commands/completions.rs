use crate::cli::{Cli, CompletionShell, CompletionsArgs};
use crate::error::AppError;
use clap::CommandFactory;
use clap_complete::{generate, shells};
use std::io;

pub fn run(args: CompletionsArgs) -> Result<(), AppError> {
    let mut command = Cli::command();
    let name = command.get_name().to_owned();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    match args.shell {
        CompletionShell::Bash => generate(shells::Bash, &mut command, &name, &mut output),
        CompletionShell::Zsh => generate(shells::Zsh, &mut command, &name, &mut output),
        CompletionShell::Fish => generate(shells::Fish, &mut command, &name, &mut output),
        CompletionShell::Elvish => generate(shells::Elvish, &mut command, &name, &mut output),
        CompletionShell::PowerShell => {
            generate(shells::PowerShell, &mut command, &name, &mut output)
        }
    }
    Ok(())
}
