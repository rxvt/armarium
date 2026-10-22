//! Converts an EPUB to AZW3 using the Metadata and Cover the EPUB itself
//! has, for trying conversion out by hand: `cargo run -p armarium-core
//! --example azw3 -- book.epub book.azw3`.

use std::path::PathBuf;

use armarium_core::extract::{self, Extracted};
use armarium_core::model::FormatKind;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [input, output] = args.as_slice() else {
        eprintln!("usage: azw3 BOOK.epub OUT.azw3");
        std::process::exit(2);
    };
    let Extracted { metadata, cover } = extract::extract(FormatKind::Epub, input)?;
    let azw3 = armarium_core::azw3::convert(input, &metadata, cover.as_deref())?;
    std::fs::write(output, azw3)?;
    Ok(())
}
