//! Test fixtures generated in code, so no real books are committed.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// A fresh directory under the system temp dir, removed on drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "armarium-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Builds a one-page PDF (300×600 pt, left half filled black) with the
/// given document info entries.
pub struct PdfFixture {
    info: Vec<(String, String)>,
}

impl PdfFixture {
    pub fn new() -> PdfFixture {
        PdfFixture { info: Vec::new() }
    }

    /// An info entry as a literal string; `value` must be plain ASCII.
    pub fn info(mut self, key: &str, value: &str) -> Self {
        let escaped = value
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        self.info.push((key.into(), format!("({escaped})")));
        self
    }

    /// An info entry as a UTF-16BE hex string, for non-ASCII text.
    pub fn info_utf16(mut self, key: &str, value: &str) -> Self {
        let mut hex = String::from("<FEFF");
        for unit in value.encode_utf16() {
            hex.push_str(&format!("{unit:04X}"));
        }
        hex.push('>');
        self.info.push((key.into(), hex));
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let content = "0 0 0 rg 0 0 150 600 re f";
        let info: String = self
            .info
            .iter()
            .map(|(k, v)| format!("/{k} {v} "))
            .collect();
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 600] /Contents 4 0 R >>".to_owned(),
            format!(
                "<< /Length {} >>\nstream\n{content}\nendstream",
                content.len()
            ),
            format!("<< {info}>>"),
        ];

        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend(format!("{} 0 obj\n{body}\nendobj\n", i + 1).bytes());
        }
        let xref = pdf.len();
        pdf.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).bytes());
        for offset in offsets {
            pdf.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        pdf.extend(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .bytes(),
        );
        pdf
    }
}

/// Builds a minimal EPUB: `mimetype`, `META-INF/container.xml` and an OPF
/// with the given metadata and manifest children, plus any extra files.
pub struct EpubFixture {
    opf_path: String,
    metadata: String,
    manifest: String,
    spine: String,
    guide: String,
    files: Vec<(String, Vec<u8>)>,
    container: bool,
}

impl EpubFixture {
    pub fn new() -> EpubFixture {
        EpubFixture {
            opf_path: "content.opf".into(),
            metadata: "<dc:title>Untitled</dc:title>".into(),
            manifest: String::new(),
            spine: String::new(),
            guide: String::new(),
            files: Vec::new(),
            container: true,
        }
    }

    pub fn opf_path(mut self, path: &str) -> Self {
        self.opf_path = path.into();
        self
    }

    /// Children of `<metadata>`; `dc:` and `opf:` prefixes are declared.
    pub fn metadata(mut self, xml: &str) -> Self {
        self.metadata = xml.into();
        self
    }

    /// Children of `<manifest>`.
    pub fn manifest(mut self, xml: &str) -> Self {
        self.manifest = xml.into();
        self
    }

    /// Children of `<spine>`.
    pub fn spine(mut self, xml: &str) -> Self {
        self.spine = xml.into();
        self
    }

    /// Children of `<guide>`, which is left out if empty.
    pub fn guide(mut self, xml: &str) -> Self {
        self.guide = xml.into();
        self
    }

    pub fn file(mut self, name: &str, bytes: &[u8]) -> Self {
        self.files.push((name.into(), bytes.into()));
        self
    }

    pub fn without_container(mut self) -> Self {
        self.container = false;
        self
    }

    pub fn build(&self) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        let mut add = |name: &str, bytes: &[u8]| {
            zip.start_file(name, stored).unwrap();
            zip.write_all(bytes).unwrap();
        };

        add("mimetype", b"application/epub+zip");
        if self.container {
            add(
                "META-INF/container.xml",
                format!(
                    r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="{}" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>"#,
                    self.opf_path
                )
                .as_bytes(),
            );
        }
        add(
            &self.opf_path,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    {}
  </metadata>
  <manifest>
    {}
  </manifest>
  <spine>
    {}
  </spine>{}
</package>"#,
                self.metadata,
                self.manifest,
                self.spine,
                if self.guide.is_empty() {
                    String::new()
                } else {
                    format!("\n  <guide>\n    {}\n  </guide>", self.guide)
                }
            )
            .as_bytes(),
        );
        for (name, bytes) in &self.files {
            add(name, bytes);
        }
        zip.finish().unwrap().into_inner()
    }
}
