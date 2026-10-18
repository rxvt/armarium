//! Compiles `data/ui/*.blp` with the system `blueprint-compiler` and bundles
//! them, plus `data/style.css`, into a GResource with `glib-compile-resources`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

const PREFIX: &str = "/io/github/rxvt/Armarium";

fn main() {
    let data = Path::new("data");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let ui_out = out.join("ui");
    std::fs::create_dir_all(&ui_out).unwrap();
    println!("cargo::rerun-if-changed=data");

    let mut blueprints: Vec<PathBuf> = std::fs::read_dir(data.join("ui"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "blp"))
        .collect();
    blueprints.sort();
    run(Command::new("blueprint-compiler")
        .arg("batch-compile")
        .arg(&ui_out)
        .arg(data.join("ui"))
        .args(&blueprints));

    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gresources>\n  <gresource prefix=\"{PREFIX}\">\n"
    );
    for blp in &blueprints {
        let ui = Path::new(blp.file_stem().unwrap()).with_extension("ui");
        writeln!(
            xml,
            "    <file compressed=\"true\" preprocess=\"xml-stripblanks\" alias=\"ui/{}\">{}</file>",
            ui.display(),
            ui_out.join(&ui).display()
        )
        .unwrap();
    }
    writeln!(
        xml,
        "    <file alias=\"style.css\">{}</file>",
        std::fs::canonicalize(data.join("style.css"))
            .unwrap()
            .display()
    )
    .unwrap();
    xml.push_str("  </gresource>\n</gresources>\n");
    let xml_path = out.join("armarium.gresource.xml");
    std::fs::write(&xml_path, xml).unwrap();

    run(Command::new("glib-compile-resources")
        .arg("--target")
        .arg(out.join("armarium.gresource"))
        .arg(&xml_path));
}

fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("cannot run {:?}: {e}", cmd.get_program()));
    assert!(status.success(), "{cmd:?} failed with {status}");
}
