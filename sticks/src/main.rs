mod apiaccess;
mod dbaccess;
mod interpreter;
mod server;

use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(name = "sticks", version, about = "Put some sticks in your HTML.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    Serve {
        #[arg(default_value = "sticks.toml")]
        config: String,
    },
    Check {
        #[arg(default_value = "sticks.toml")]
        config: String,
    },
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Check { config } => {
            let config_path = PathBuf::from(&config);
            let mut base_dir = config_path.parent().unwrap_or(Path::new("")).to_path_buf();
            if base_dir.as_os_str().is_empty() {
                base_dir = PathBuf::from(".");
            }

            let content = match std::fs::read_to_string(&config_path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("❌ Configuration read error: {e}");
                    std::process::exit(1);
                }
            };

            let cfg: server::Config = match toml::from_str(&content) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("❌ TOML syntax error: {e}");
                    std::process::exit(1);
                }
            };

            let site = match cfg.site.first() {
                Some(s) => s,
                None => {
                    eprintln!("❌ No [[site]] block declared in {config}");
                    std::process::exit(1);
                }
            };

            let pub_dir = base_dir.join(&site.public_dir);
            match server::check_templates(&pub_dir) {
                Ok(()) => {
                    println!("✅ Static check passed: all templates in {:?} are valid.", pub_dir);
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("❌ Check failed:\n{e}");
                    std::process::exit(1);
                }
            }
        }
        Command::Serve { config } => {
            if let Err(e) = server::run_server(&config).await {
                eprintln!("Fatal server error: {e}");
                std::process::exit(1);
            }
        }
    }
}