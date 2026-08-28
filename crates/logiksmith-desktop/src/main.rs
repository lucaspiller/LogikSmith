use clap::Parser;
use logiksmith_desktop::{load_config, run};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "logiksmith",
    version,
    about = "KNX automation proof of concept"
)]
struct Args {
    /// Path to the TOML configuration file.
    #[arg(long, value_name = "PATH")]
    config: PathBuf,
    /// Path to the TOML automation document.
    #[arg(long, value_name = "PATH")]
    automation: PathBuf,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let config = match load_config(&args.config, &args.automation) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("configuration error: {error}");
            return std::process::ExitCode::from(2);
        }
    };

    if let Err(error) = run(config).await {
        eprintln!("logiksmith failed: {error}");
        return std::process::ExitCode::from(1);
    }
    std::process::ExitCode::SUCCESS
}
