use markdown_converter::convert::{ConvertRequest, convert_file};
use markdown_converter::pandoc::Pandoc;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Copies a fixture into a fresh temp dir under `as_name` (HTML also gets `local.png`).
fn setup(fixture_name: &str, as_name: &str) -> (TempDir, PathBuf) {
    let d = TempDir::new().unwrap();
    let dest = d.path().join(as_name);
    std::fs::copy(fixture(fixture_name), &dest).unwrap();
    if fixture_name.ends_with(".html") {
        std::fs::copy(fixture("local.png"), d.path().join("local.png")).unwrap();
    }
    (d, dest)
}

fn pandoc() -> Pandoc {
    Pandoc::locate().unwrap()
}

fn request(d: &TempDir, source: PathBuf) -> ConvertRequest {
    ConvertRequest {
        source,
        output_dir: d.path().into(),
    }
}

/// Every Image `src` in a Markdown file, as parsed by pandoc.
fn image_srcs(md: &Path) -> Vec<String> {
    let out = std::process::Command::new(pandoc().path())
        .args(["-f", "gfm", "-t", "json"])
        .arg(md)
        .output()
        .unwrap();
    assert!(out.status.success());
    let ast: Value = serde_json::from_slice(&out.stdout).unwrap();
    let mut srcs = Vec::new();
    collect_srcs(&ast, &mut srcs);
    srcs
}

fn collect_srcs(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(items) => items.iter().for_each(|i| collect_srcs(i, out)),
        Value::Object(o) => {
            if o.get("t").and_then(Value::as_str) == Some("Image") {
                if let Some(src) = o["c"][2][0].as_str() {
                    out.push(src.to_string());
                }
            }
            o.values().for_each(|c| collect_srcs(c, out));
        }
        _ => {}
    }
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn docx_with_pictures() {
    let (d, src) = setup("with-pictures.docx", "report.docx");
    let o = convert_file(&pandoc(), &request(&d, src)).unwrap();
    assert_eq!(o.md_path, d.path().join("report-converted.md"));
    assert_eq!(o.pictures, 2);
    let md = std::fs::read_to_string(&o.md_path).unwrap();
    assert!(md.contains("# Quarterly Report") && md.contains("| Region"));
    assert!(md.contains("![Sales Chart Q3](report-assets/sales-chart-q3.png)"));
    assert!(md.contains("report-assets/image-002.png"));
    assert!(!md.contains("<img") && !md.contains("<figure"));
}

#[test]
fn complex_table_kept_as_html_table() {
    let (d, src) = setup("complex-table.docx", "table.docx");
    let o = convert_file(&pandoc(), &request(&d, src)).unwrap();
    let md = std::fs::read_to_string(&o.md_path).unwrap();
    assert!(!md.contains("[TABLE]"), "{md}");
    assert!(
        md.contains("First para.") && md.contains("Second para."),
        "{md}"
    );
    assert!(md.contains("Alpha"), "{md}");
}

#[test]
fn no_html_outside_complex_tables() {
    for name in [
        "with-pictures.docx",
        "slides.pptx",
        "two-sheets.xlsx",
        "sample.odt",
        "sample.rtf",
        "sample.epub",
        "page.html",
    ] {
        let (d, src) = setup(name, name);
        let o = convert_file(&pandoc(), &request(&d, src)).unwrap();
        let md = std::fs::read_to_string(&o.md_path).unwrap();
        for tag in [
            "<img", "<figure", "<span", "<div", "<a ", "<sup", "<sub", "<u>",
        ] {
            assert!(!md.contains(tag), "{name}: {tag} in:\n{md}");
        }
    }
}

#[test]
fn docx_without_pictures_has_no_assets_dir() {
    let (d, src) = setup("no-pictures.docx", "report.docx");
    let o = convert_file(&pandoc(), &request(&d, src)).unwrap();
    assert_eq!(o.pictures, 0);
    assert_eq!(o.assets_dir, None);
    assert!(!d.path().join("report-assets").exists());
}

#[test]
fn second_run_gets_index_one() {
    let (d, src) = setup("with-pictures.docx", "report.docx");
    convert_file(&pandoc(), &request(&d, src.clone())).unwrap();
    let o = convert_file(&pandoc(), &request(&d, src)).unwrap();
    assert_eq!(o.md_path, d.path().join("report-converted (1).md"));
    let md = std::fs::read_to_string(&o.md_path).unwrap();
    assert!(md.contains("report-assets-(1)/"));
    assert!(d.path().join("report-assets-(1)").is_dir());
}

#[test]
fn pptx_names_from_alt_not_generic_title() {
    let (d, src) = setup("slides.pptx", "slides.pptx");
    let o = convert_file(&pandoc(), &request(&d, src)).unwrap();
    let names = file_names(&o.assets_dir.unwrap());
    assert!(names.contains(&"red.png".to_string()), "{names:?}");
    assert!(names.contains(&"green.png".to_string()), "{names:?}");
    assert!(
        !names.iter().any(|n| n.starts_with("picture-1")),
        "{names:?}"
    );
}

#[test]
fn xlsx_each_sheet_heading_and_table() {
    let (d, src) = setup("two-sheets.xlsx", "book.xlsx");
    let o = convert_file(&pandoc(), &request(&d, src)).unwrap();
    let md = std::fs::read_to_string(&o.md_path).unwrap();
    for needle in ["## Revenue", "| North", "## Costs", "| Rent"] {
        assert!(md.contains(needle), "missing {needle:?} in:\n{md}");
    }
}

#[test]
fn odt_rtf_epub_convert_with_pictures() {
    for name in ["sample.odt", "sample.rtf", "sample.epub"] {
        let (d, src) = setup(name, name);
        let o =
            convert_file(&pandoc(), &request(&d, src)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert!(o.pictures >= 1, "{name}: no pictures");
        let srcs = image_srcs(&o.md_path);
        assert!(!srcs.is_empty(), "{name}: no images in output");
        for s in srcs {
            assert!(d.path().join(&s).exists(), "{name}: {s} does not exist");
        }
    }
}

#[test]
fn html_local_copied_remote_kept_missing_warned() {
    let (d, src) = setup("page.html", "page.html");
    let o = convert_file(&pandoc(), &request(&d, src)).unwrap();
    assert_eq!(o.pictures, 1);
    let md = std::fs::read_to_string(&o.md_path).unwrap();
    assert!(md.contains("https://example.com/remote.png"));
    assert_eq!(
        o.warnings,
        vec!["Picture not found: missing.png".to_string()]
    );
}

#[test]
fn convert_unicode_name_links_resolve() {
    let (d, src) = setup("with-pictures.docx", "Báo cáo Q3 (final).docx");
    let o = convert_file(&pandoc(), &request(&d, src)).unwrap();
    assert_eq!(o.md_path, d.path().join("Báo cáo Q3 (final)-converted.md"));
    let srcs = image_srcs(&o.md_path);
    assert!(!srcs.is_empty());
    for s in srcs {
        assert!(d.path().join(&s).exists(), "{s} does not exist");
    }
}

#[test]
fn corrupt_file_fails_and_cleans_up() {
    let (d, src) = setup("corrupt.docx", "bad.docx");
    let e = convert_file(&pandoc(), &request(&d, src)).unwrap_err();
    assert!(!e.reason.is_empty());
    let leftovers: Vec<String> = file_names(d.path())
        .into_iter()
        .filter(|n| n.contains("-converted") || n.contains("-assets"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn convert_missing_source_fails() {
    let d = TempDir::new().unwrap();
    let e = convert_file(&pandoc(), &request(&d, d.path().join("gone.docx"))).unwrap_err();
    assert_eq!(e.reason, "File not found");
}

#[test]
fn convert_missing_output_dir_fails_cleanly() {
    let (d, src) = setup("no-pictures.docx", "report.docx");
    let req = ConvertRequest {
        source: src,
        output_dir: d.path().join("nope"),
    };
    let e = convert_file(&pandoc(), &req).unwrap_err();
    assert!(
        e.reason.starts_with("Output folder not found"),
        "{}",
        e.reason
    );
    assert!(!d.path().join("nope").exists());
}
