//! MHL Verify CLI — Standalone ASC MHL verification tool.
//!
//! A zero-dependency, cross-platform command-line tool for verifying
//! ASC MHL hash lists. Post-production houses can use this to verify
//! delivered media without installing the full DIT System.
//!
//! Usage:
//!   mhl-verify <path-to-mhl-file>
//!   mhl-verify --directory <path>

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "mhl-verify",
    about = "Verify ASC MHL hash lists for media integrity",
    version
)]
struct Args {
    /// Path to an ASC MHL file or directory containing MHL files
    #[arg()]
    path: String,

    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    println!("MHL Verify CLI v{}", env!("CARGO_PKG_VERSION"));
    println!("Verifying: {}", args.path);

    // TODO: Implement MHL parsing and verification
    println!("MHL verification engine is under development.");

    Ok(())
}
