use clap::Parser;
use mi_merge::cli::Cli;
use mi_merge::runner;
use std::process::ExitCode;

fn main() -> ExitCode {
    let cli = Cli::parse();
    match runner::run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}
