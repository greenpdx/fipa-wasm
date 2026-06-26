//! Build tooling for the UNL stack.
//!
//! Run from anywhere in the workspace:
//!   cargo run -p xtask -- fetch-wordnet [--force]
//!
//! Subcommands:
//!   fetch-wordnet   Download + extract the Princeton WordNet 3.1 database files
//!                   into data/kb-seed/wordnet-3.1/. This is the open seed the
//!                   WordNetKb builder reads (manifest §4.3).

use std::error::Error;
use std::path::{Path, PathBuf};

/// Princeton WordNet 3.1 database files (index.*, data.*, *.exc). 3.1 was only
/// released as this database tarball (no separate binary package).
const WORDNET_URL: &str = "https://wordnetcode.princeton.edu/wn3.1.dict.tar.gz";

fn main() {
    let mut args = std::env::args().skip(1);
    let cmd = args.next();
    let rest: Vec<String> = args.collect();
    let result = match cmd.as_deref() {
        Some("fetch-wordnet") => fetch_wordnet(rest.iter().any(|a| a == "--force")),
        Some(other) => {
            eprintln!("unknown subcommand: {other}");
            usage();
            std::process::exit(2);
        }
        None => {
            usage();
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn usage() {
    eprintln!("usage: cargo run -p xtask -- <command>");
    eprintln!("  fetch-wordnet [--force]   download + extract WordNet 3.1 to data/kb-seed/");
}

/// The workspace root, derived from this crate's location (xtask/ -> ..), so the
/// command works regardless of the current directory.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent directory")
        .to_path_buf()
}

fn fetch_wordnet(force: bool) -> Result<(), Box<dyn Error>> {
    let dest = workspace_root().join("data/kb-seed/wordnet-3.1");
    // The tarball extracts into a `dict/` subdirectory.
    let marker = dest.join("dict/data.noun");
    if marker.exists() && !force {
        println!(
            "WordNet 3.1 already present at {} (use --force to re-download)",
            dest.display()
        );
        return Ok(());
    }

    std::fs::create_dir_all(&dest)?;
    println!("Downloading {WORDNET_URL}");
    let resp = ureq::get(WORDNET_URL).call()?;
    let reader = resp.into_reader();

    println!("Extracting to {} ...", dest.display());
    let gz = flate2::read::GzDecoder::new(reader);
    let mut archive = tar::Archive::new(gz);
    archive.unpack(&dest)?;

    verify(&dest)?;
    Ok(())
}

/// Sanity-check the extracted tree and report what landed.
fn verify(dest: &Path) -> Result<(), Box<dyn Error>> {
    let dict = dest.join("dict");
    let expected = [
        "data.noun",
        "data.verb",
        "data.adj",
        "data.adv",
        "index.noun",
        "index.verb",
    ];
    let mut missing = Vec::new();
    for name in expected {
        if !dict.join(name).exists() {
            missing.push(name);
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "extraction incomplete; missing under {}: {:?}",
            dict.display(),
            missing
        )
        .into());
    }

    // Count noun synsets (data lines, skipping the licence header lines that
    // start with two spaces).
    let data_noun = std::fs::read_to_string(dict.join("data.noun"))?;
    let synsets = data_noun.lines().filter(|l| !l.starts_with("  ")).count();
    println!(
        "OK: WordNet 3.1 extracted to {} ({} noun synsets)",
        dict.display(),
        synsets
    );
    Ok(())
}
