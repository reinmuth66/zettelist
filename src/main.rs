mod config;

use clap::Parser;

/// Zettelkasten on Omnigraph の書き込みと整理の道具
#[derive(Parser)]
#[command(name = "zettelist", version)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}
