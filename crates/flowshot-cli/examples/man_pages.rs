//! Man-page generation harness (plan todo 35; examples double as QA
//! harnesses per todo 1). Writes `flowshot.1` plus one page per
//! subcommand (rendered from the clap surface via `clap_mangen`; the
//! top-level page cross-references them) and the hand-authored
//! `flowshot-config.5` into a directory.
//!
//! Usage: `cargo run -p flowshot-cli --example man_pages -- [OUT_DIR]`
//! (default `OUT_DIR`: `target/man`). Packaging (todo 39) installs the
//! output into the mandb hierarchy.

use std::path::PathBuf;

use clap::CommandFactory;
use flowshot_cli::args::Cli;
use flowshot_cli::config_man::FLOWSHOT_CONFIG_ROFF;

fn main() -> anyhow::Result<()> {
    let out_dir = std::env::args_os()
        .nth(1)
        .map_or_else(|| PathBuf::from("target/man"), PathBuf::from);
    std::fs::create_dir_all(&out_dir)?;

    clap_mangen::generate_to(Cli::command(), &out_dir)?;
    for entry in std::fs::read_dir(&out_dir)? {
        println!("{}", entry?.path().display());
    }

    let config_5 = out_dir.join("flowshot-config.5");
    std::fs::write(&config_5, FLOWSHOT_CONFIG_ROFF)?;
    println!("{}", config_5.display());
    Ok(())
}
