mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use feedforge::Outcome;
use support::parse;

const CATALOG: &str = include_str!("fixtures/catalog.json");

const SKIP_CATALOG: &str = r#"{
  "shop": {
    "name": "Example Shop",
    "url": "https://example.test",
    "email": "shop@example.test"
  },
  "products": [
    {
      "id": "SKU-1",
      "title": "Kávovar",
      "description": "Příliš žluťoučký kůň.",
      "price_minor": 12345,
      "currency": "CZK",
      "availability": "in_stock",
      "url": "https://example.test/p/sku-1",
      "image_urls": ["https://example.test/img/sku-1.jpg"],
      "category_path": ["Domov"]
    },
    {
      "id": "SKU-2",
      "title": "Bez popisu",
      "price_minor": 500,
      "currency": "CZK",
      "availability": "out_of_stock",
      "url": "https://example.test/p/sku-2",
      "image_urls": ["https://example.test/img/sku-2.jpg"],
      "category_path": ["Domov"]
    }
  ]
}"#;

fn test_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("remove old test directory");
    }
    fs::create_dir_all(&dir).expect("create test directory");
    dir
}

fn write_config(dir: &Path, portal: &str, output: &Path, extra: &str) -> PathBuf {
    let path = dir.join("feed.toml");
    let text = format!(
        "portal = \"{portal}\"\noutput = \"{}\"\ncurrency = \"CZK\"\n{extra}",
        output.display()
    );
    fs::write(&path, text).expect("write config");
    path
}

fn write_catalog(dir: &Path, content: &str) -> PathBuf {
    let path = dir.join("catalog.json");
    fs::write(&path, content).expect("write catalog");
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_feedforge"))
        .args(args)
        .output()
        .expect("run feedforge")
}

#[test]
fn cli_writes_feed_and_report() {
    let dir = test_dir("cli_writes_feed_and_report");
    let output = dir.join("feed.xml");
    let report = dir.join("report.json");
    let config = write_config(&dir, "heureka", &output, "");
    let catalog = write_catalog(&dir, CATALOG);
    let result = run(&[
        "--config",
        config.to_str().unwrap(),
        "--input",
        catalog.to_str().unwrap(),
        "--report",
        report.to_str().unwrap(),
    ]);
    assert_eq!(
        result.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    let xml = fs::read_to_string(&output).expect("read feed");
    let root = parse(&xml);
    assert_eq!(root.name, "SHOP");
    assert_eq!(root.children_named("SHOPITEM").count(), 2);
    let outcome: Outcome = serde_json::from_str(&fs::read_to_string(&report).expect("read report"))
        .expect("report JSON");
    assert_eq!(outcome.written, 2);
    assert_eq!(outcome.skipped_count(), 0);
}

#[test]
fn cli_strict_exit_code_follows_skips() {
    let dir = test_dir("cli_strict_exit_code_follows_skips");
    let output = dir.join("feed.xml");
    let report = dir.join("report.json");
    let config = write_config(&dir, "heureka", &output, "");
    let catalog = write_catalog(&dir, SKIP_CATALOG);
    let relaxed = run(&[
        "--config",
        config.to_str().unwrap(),
        "--input",
        catalog.to_str().unwrap(),
        "--report",
        report.to_str().unwrap(),
    ]);
    assert_eq!(relaxed.status.code(), Some(0));
    let outcome: Outcome = serde_json::from_str(&fs::read_to_string(&report).expect("read report"))
        .expect("report JSON");
    assert_eq!(outcome.written, 1);
    assert_eq!(outcome.skipped_count(), 1);
    assert_eq!(
        outcome.skipped[0].reasons,
        vec!["missing_field:description"]
    );
    let strict = run(&[
        "--config",
        config.to_str().unwrap(),
        "--input",
        catalog.to_str().unwrap(),
        "--strict",
    ]);
    assert_eq!(strict.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&strict.stderr).contains("SKU-2"));
}

#[test]
fn cli_output_flag_overrides_config() {
    let dir = test_dir("cli_output_flag_overrides_config");
    let configured = dir.join("configured.xml");
    let override_path = dir.join("override.xml");
    let config = write_config(&dir, "google", &configured, "");
    let catalog = write_catalog(&dir, CATALOG);
    let result = run(&[
        "--config",
        config.to_str().unwrap(),
        "--input",
        catalog.to_str().unwrap(),
        "--output",
        override_path.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(0));
    assert!(override_path.exists());
    assert!(!configured.exists());
    let root = parse(&fs::read_to_string(&override_path).expect("read feed"));
    assert_eq!(root.name, "rss");
}

#[test]
fn cli_errors_exit_with_code_two() {
    let dir = test_dir("cli_errors_exit_with_code_two");
    let catalog = write_catalog(&dir, CATALOG);
    let bad_config = dir.join("bad.toml");
    fs::write(
        &bad_config,
        "portal = \"heureka\"\noutput = \"feed.xml\"\ncurrency = \"EUR\"\n",
    )
    .expect("write bad config");
    let result = run(&[
        "--config",
        bad_config.to_str().unwrap(),
        "--input",
        catalog.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(2));

    let config = write_config(&dir, "heureka", &dir.join("feed.xml"), "");
    let broken = write_catalog(&dir, "{ not json");
    let result = run(&[
        "--config",
        config.to_str().unwrap(),
        "--input",
        broken.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(2));

    let result = run(&[
        "--config",
        config.to_str().unwrap(),
        "--input",
        dir.join("missing.json").to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(2));
}

#[test]
fn cli_output_is_deterministic() {
    let dir = test_dir("cli_output_is_deterministic");
    let first = dir.join("first.xml");
    let second = dir.join("second.xml");
    let config = write_config(&dir, "zbozi", &first, "");
    let catalog = write_catalog(&dir, CATALOG);
    let result = run(&[
        "--config",
        config.to_str().unwrap(),
        "--input",
        catalog.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(0));
    let result = run(&[
        "--config",
        config.to_str().unwrap(),
        "--input",
        catalog.to_str().unwrap(),
        "--output",
        second.to_str().unwrap(),
    ]);
    assert_eq!(result.status.code(), Some(0));
    let first_bytes = fs::read(&first).expect("read first feed");
    let second_bytes = fs::read(&second).expect("read second feed");
    assert_eq!(first_bytes, second_bytes);
}
