//! Construction contracts for call sites and cut annotations.
//!
//! A call site is built through a constructor that pairs the variant with the
//! payload it may carry, and a cut annotation endpoint is a typed portable
//! path. Both keep the serialized shape they had.

use std::path::{Path, PathBuf};

use meta_ast::LangId;

#[test]
fn call_site_construction_pairs_variant_and_payload() {
    use meta_ast::deploy::scanner::{CallSite, CallSiteVariant};

    let load = CallSite::load(
        PathBuf::from("main.py"),
        LangId::Python,
        CallSiteVariant::LoadFromFile,
        Some("node".to_string()),
        vec!["index.js".to_string()],
        None,
        1.0,
    );
    assert_eq!(load.variant, CallSiteVariant::LoadFromFile);
    assert_eq!(load.function_name, None);
    assert_eq!(load.scripts, ["index.js"]);
    assert_eq!(load.target_lang.as_deref(), Some("node"));

    let invocation = CallSite::call(
        PathBuf::from("main.py"),
        LangId::Python,
        "multiply".to_string(),
        false,
        None,
        1.0,
    );
    assert_eq!(invocation.variant, CallSiteVariant::ClientCall);
    assert_eq!(invocation.function_name.as_deref(), Some("multiply"));
    assert!(invocation.scripts.is_empty());
    assert_eq!(invocation.target_lang, None);

    let untargeted =
        CallSite::call_without_target(PathBuf::from("main.py"), LangId::Python, true, None, 1.0);
    assert_eq!(untargeted.variant, CallSiteVariant::ClientCall);
    assert_eq!(untargeted.function_name, None);
    assert!(untargeted.is_async);

    let value = serde_json::to_value(&invocation).unwrap();
    assert_eq!(value["variant"], "ClientCall");
    assert_eq!(value["scripts"], serde_json::json!([]));
    assert!(value["target_lang"].is_null());
}

/// The scanner builds sites through the constructors, never with a literal.
#[test]
fn the_scanner_uses_the_constructors() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/deploy/scanner.rs");
    let scanner = std::fs::read_to_string(&path).unwrap();
    let literal_lines: Vec<&str> = scanner
        .lines()
        .filter(|line| {
            line.contains("CallSite {")
                && !line.contains("pub struct CallSite")
                && !line.contains("impl CallSite")
                && !line.contains("-> CallSite")
        })
        .collect();
    assert!(
        literal_lines.is_empty(),
        "the scanner must build call sites through the constructors, found {literal_lines:?}"
    );
    assert!(scanner.contains("CallSite::load("));
    assert!(scanner.contains("CallSite::call("));
}

#[test]
fn cut_annotation_endpoints_are_portable_paths() {
    use meta_ast::deploy::cut::{CutAnnotation, CutReason, PortablePath};
    use meta_ast::model::FileId;

    let path = Path::new("src/a b.py");
    let portable = PortablePath::from_path(path);
    assert_eq!(portable.as_str(), meta_ast::input::portable_path(path));

    let id = FileId::new(7);
    assert!(id.is_some(), "7 is a valid file id");
    let anchor = PortablePath::anchor(id.unwrap());
    assert_eq!(anchor.as_str(), "file#7");

    let annotation = CutAnnotation {
        from_file: portable,
        to_file: anchor,
        cut_reason: CutReason::CrossLanguageScc,
        original_confidence: 0.4,
    };
    let value = serde_json::to_value(&annotation).unwrap();
    assert_eq!(value["from_file"], meta_ast::input::portable_path(path));
    assert_eq!(value["to_file"], "file#7");
}
