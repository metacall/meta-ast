//! The model stores the bare import specifier, and undecodable text is a
//! diagnostic instead of a placeholder.
//!
//! Import statements wrap a specifier in a string literal or angle brackets.
//! Every consumer wants the bare form, so the wrap characters are trimmed
//! where the model value is produced, in one place for all languages.

use std::path::{Path, PathBuf};

use meta_ast::error::Severity;
use meta_ast::extractor::ExtractOptions;
use meta_ast::{ExtractionResult, FileExtraction};

fn fixtures(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn extract_root(root: &Path) -> ExtractionResult {
    let files = meta_ast::input::discover_files(root, None).unwrap();
    assert!(!files.is_empty(), "{} holds no source file", root.display());
    meta_ast::extractor::extract_with_options(
        &files,
        &ExtractOptions {
            skip_imports_and_refs: false,
        },
    )
}

fn file<'a>(result: &'a ExtractionResult, name: &str) -> &'a FileExtraction {
    result
        .files
        .iter()
        .find(|file| file.path.file_name().is_some_and(|found| found == name))
        .unwrap_or_else(|| {
            let seen: Vec<String> = result
                .files
                .iter()
                .map(|file| file.path.display().to_string())
                .collect();
            panic!("{name} is not in the extraction result, saw {seen:?}")
        })
}

fn specifiers(file: &FileExtraction) -> Vec<&str> {
    file.imports
        .iter()
        .map(|import| import.import_specifier.as_str())
        .collect()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("meta_ast_import_hygiene_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn import_specifiers_are_bare() {
    let result = extract_root(&fixtures("import_hygiene"));

    let expected = [
        ("app.js", "react"),
        ("app.js", "./helper"),
        ("module.ts", "@angular/core"),
        ("constants.py", "json"),
        ("native.c", "stdio.h"),
        ("native.c", "config.h"),
    ];
    for (name, specifier) in expected {
        let file = file(&result, name);
        assert!(
            file.imports
                .iter()
                .any(|import| import.import_specifier == specifier),
            "{name} must carry the bare specifier {specifier:?}, found {:?}",
            specifiers(file)
        );
    }

    for file in &result.files {
        for import in &file.imports {
            let specifier = import.import_specifier.as_str();
            assert!(
                !specifier.starts_with(['\'', '"', '<']),
                "{} carries an opening wrap character in {specifier:?}",
                file.path.display()
            );
            assert!(
                !specifier.ends_with(['\'', '"', '>']),
                "{} carries a closing wrap character in {specifier:?}",
                file.path.display()
            );
            assert!(
                !specifier.is_empty(),
                "{} carries an empty specifier",
                file.path.display()
            );
        }
    }
}

fn rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_placeholder_is_gone_from_the_model_and_the_source() {
    let result = extract_root(&fixtures("import_hygiene"));
    for file in &result.files {
        for import in &file.imports {
            assert!(!import.import_specifier.contains("invalid-utf8"));
        }
        for symbol in &file.symbols {
            assert!(!symbol.name.contains("invalid-utf8"));
        }
    }

    let mut sources = Vec::new();
    rust_sources(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut sources,
    );
    assert!(sources.len() > 20, "the source walk found no modules");
    for path in sources {
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("<invalid-utf8>"),
            "{} still carries the placeholder",
            path.display()
        );
    }
}

#[test]
fn an_undecodable_specifier_is_a_warning() {
    let root = temp_dir("undecodable");
    let path = root.join("broken.js");
    std::fs::write(&path, b"import { x } from '\xff\xfe';\n").unwrap();

    let result = extract_root(&root);
    assert_eq!(result.files.len(), 1);
    let file = &result.files[0];

    let warnings: Vec<&meta_ast::Diagnostic> = file
        .diagnostics
        .iter()
        .filter(|diagnostic| matches!(diagnostic.severity, Severity::Warning))
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "expected one warning for the undecodable specifier, got {:?}",
        file.diagnostics
    );
    let warning = warnings[0];
    assert_eq!(warning.path, path);
    let range = warning
        .source_range
        .as_ref()
        .unwrap_or_else(|| panic!("the warning must carry a range: {:?}", warning.message));
    assert!(
        range.byte_end > range.byte_start,
        "the warning range must not be empty"
    );

    assert!(
        file.imports.is_empty(),
        "the undecodable import is dropped, not stored, found {:?}",
        specifiers(file)
    );

    let _ = std::fs::remove_dir_all(&root);
}
