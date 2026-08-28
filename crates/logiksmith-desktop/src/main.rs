use clap::Parser;
use logiksmith_desktop::{load_config, load_simulation_config, run, run_simulation_only};
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
    /// Run the browser editor and Lua simulator without starting KNX/XKNX.
    #[arg(long)]
    simulation_only: bool,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();
    let config = match if args.simulation_only {
        load_simulation_config(&args.config, &args.automation)
    } else {
        load_config(&args.config, &args.automation)
    } {
        Ok(config) => config,
        Err(error) => {
            eprintln!("configuration error: {error}");
            return std::process::ExitCode::from(2);
        }
    };

    let result = if args.simulation_only {
        run_simulation_only(config).await
    } else {
        run(config).await
    };
    if let Err(error) = result {
        eprintln!("logiksmith failed: {error}");
        return std::process::ExitCode::from(1);
    }
    std::process::ExitCode::SUCCESS
}
