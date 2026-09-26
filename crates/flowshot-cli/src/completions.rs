//! Shell completion generation (plan todo 35: bash/zsh/fish/elvish/pwsh via
//! `clap_complete`, nushell via `clap_complete_nushell`; wayshot precedent,
//! draft F6). Generated scripts are the artifact: they go to stdout for
//! shell-level redirection or packaging install.

use std::io::{self, Write};

use clap::CommandFactory;
use clap_complete::{Shell, generate};
use clap_complete_nushell::Nushell;

use crate::args::{Cli, CompletionShell};
use crate::exit::CliError;

/// The binary name completions are generated for.
pub const BIN_NAME: &str = "flowshot";

/// Generates the completion script for `shell` onto stdout.
///
/// # Errors
///
/// [`CliError::Io`] when stdout cannot be flushed (broken pipe).
pub fn generate_to_stdout(shell: CompletionShell) -> Result<(), CliError> {
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());
    let mut command = Cli::command();
    match shell {
        CompletionShell::Bash => generate(Shell::Bash, &mut command, BIN_NAME, &mut writer),
        CompletionShell::Zsh => generate(Shell::Zsh, &mut command, BIN_NAME, &mut writer),
        CompletionShell::Fish => generate(Shell::Fish, &mut command, BIN_NAME, &mut writer),
        CompletionShell::Elvish => generate(Shell::Elvish, &mut command, BIN_NAME, &mut writer),
        CompletionShell::Pwsh => generate(Shell::PowerShell, &mut command, BIN_NAME, &mut writer),
        CompletionShell::Nushell => generate(Nushell, &mut command, BIN_NAME, &mut writer),
    }
    writer.flush().map_err(CliError::Io)
}
