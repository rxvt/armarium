//! Imports files or folders into a Library from the command line, for
//! development: `cargo run -p armarium-core --example import -- LIBRARY PATH...`

use std::path::PathBuf;

use armarium_core::Library;
use armarium_core::import::Outcome;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let root = args.next().ok_or("usage: import LIBRARY PATH...")?;
    let paths: Vec<PathBuf> = args.collect();

    let mut library = Library::open(root)?;
    let report = library.import(&paths, |p| {
        eprintln!("[{}/{}] {}", p.done + 1, p.total, p.current.display())
    })?;
    for (path, outcome) in &report.items {
        match outcome {
            Outcome::Added(id) => println!("added   {} (book {})", path.display(), id.0),
            Outcome::Merged(id) => println!("merged  {} (book {})", path.display(), id.0),
            Outcome::Skipped(reason) => println!("skipped {}: {reason}", path.display()),
        }
    }
    println!(
        "{} added · {} merged · {} skipped",
        report.added(),
        report.merged(),
        report.skipped()
    );
    Ok(())
}
