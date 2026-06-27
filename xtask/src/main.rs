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
        Some("fetch-aesop") => fetch_aesop(),
        Some("build-kb") => build_kb(),
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
    eprintln!("  fetch-aesop               download the AESOP UNL corpus to data/corpus/aesop/");
    eprintln!("  build-kb                  compile the embedded SledKb from the WordNet seed");
}

/// Compile the embedded knowledge base (SledKb) from the WordNet 3.1 seed.
fn build_kb() -> Result<(), Box<dyn Error>> {
    use unl_kb::{SledKb, WordNetKb};

    let root = workspace_root();
    let dict = root.join("data/kb-seed/wordnet-3.1/dict");
    let out = root.join("data/kb-seed/unl-kb.sled");
    if !dict.join("data.noun").exists() {
        return Err(format!(
            "WordNet not found at {} — run `cargo run -p xtask -- fetch-wordnet` first",
            dict.display()
        )
        .into());
    }
    println!("Compiling embedded KB from {} ...", dict.display());
    let wordnet = WordNetKb::open(&dict)?;
    let (_kb, stats) = SledKb::build_from_wordnet(&wordnet, &out)?;
    println!(
        "OK: {} concepts, {} lemmas -> {}",
        stats.concepts,
        stats.lemmas,
        out.display()
    );
    Ok(())
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

// ---------------------------------------------------------------------------
// AESOP UNL corpus
// ---------------------------------------------------------------------------

/// Languages for which the unlarchive.org mirror returns non-empty AESOP graphs.
const AESOP_LANGS: &[&str] = &["en", "fr", "es", "ru", "pt", "it"];

fn aesop_url(lang: &str) -> String {
    format!("https://unlarchive.org/unlarium/corpus/export_corpus.php?project=aa1&lang={lang}&unl=ucl")
}

/// Download the AESOP corpus (the surviving UNLarium example corpus) for each
/// available language, strip the HTML wrapper, and write clean `[D]...[S]...`
/// UNL text to data/corpus/aesop/. The corpus carries no explicit open licence,
/// so it is fetched on demand and gitignored, not vendored.
fn fetch_aesop() -> Result<(), Box<dyn Error>> {
    let dir = workspace_root().join("data/corpus/aesop");
    std::fs::create_dir_all(&dir)?;
    let mut total = 0usize;
    for &lang in AESOP_LANGS {
        print!("Fetching AESOP [{lang}] ... ");
        let body = ureq::get(&aesop_url(lang)).call()?.into_string()?;
        let cleaned = clean_corpus_html(&body);
        let sentences = cleaned.matches("[S:").count();
        total += sentences;
        let path = dir.join(format!("aesop_{lang}.unl"));
        std::fs::write(&path, &cleaned)?;
        println!("{sentences} sentences -> {}", path.display());
    }
    println!("OK: {total} sentences across {} languages", AESOP_LANGS.len());
    Ok(())
}

/// Turn the HTML-wrapped export into clean UNL text: `<br />` -> newline, strip
/// remaining tags, decode the handful of entities that appear, and keep just the
/// `[D ... [/D]` document envelope.
fn clean_corpus_html(raw: &str) -> String {
    let with_newlines = raw
        .replace("<br />", "\n")
        .replace("<br/>", "\n")
        .replace("<br>", "\n");

    let mut text = String::with_capacity(with_newlines.len());
    let mut in_tag = false;
    for ch in with_newlines.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }

    let text = text
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");

    let start = text.find("[D").unwrap_or(0);
    let end = text.find("[/D]").map(|e| e + 4).unwrap_or(text.len());
    let mut out = text[start..end].trim().to_string();
    out.push('\n');
    out
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
