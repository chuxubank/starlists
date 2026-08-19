mod cli;
mod cmd;
mod config;
mod db;
mod github;
mod ids;
mod output;
mod proposal;

use clap::Parser;
use cli::Cli;
use config::{AppConfig, Paths};

fn main() {
    let cli = Cli::parse();
    let format = cli.format();
    let result = (|| {
        let paths = Paths::resolve(cli.config.clone(), cli.db.clone())?;
        let cfg = AppConfig::load(paths)?;
        cmd::run(&cfg, format, cli.command)
    })();

    if let Err(err) = result {
        let code = output::fail(format, &err);
        std::process::exit(code);
    }
}
