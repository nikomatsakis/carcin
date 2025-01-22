use clap::Parser;

use crate::util;

#[derive(clap::Parser)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(clap::Subcommand)]
enum CliCommand {
    Jar,
}

/// Main function from the binary
pub fn bin_main() -> anyhow::Result<()> {
    let java_class_files = util::make_java_class_files_directory()?;
    let cli = Cli::try_parse()?;
    match cli.command {
        CliCommand::Jar => {
            
        }
    }
    Ok(())
}