use std::process::ExitCode;

use clap::Parser;
use harbor::cli::Cli;

fn main() -> ExitCode {
    #[cfg(unix)]
    unsafe {
        // Restore the conventional CLI behaviour when output is piped into a
        // program that exits early (for example `harbor list | head`).
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let cli = Cli::parse();
    match harbor::cli::run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let message = harbor::format::sanitize(&error.to_string());
            eprintln!("harbor: {}", harbor::format::truncate(&message, 1000));
            ExitCode::FAILURE
        }
    }
}
