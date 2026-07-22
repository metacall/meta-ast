//! Parallel file extraction orchestration.
//!
//! Uses rayon `par_iter` to read, parse, and extract symbols/imports/
//! references across files concurrently. Each file is processed
//! independently; errors are accumulated as diagnostics per file.
//!
//! Set `ExtractOptions::skip_imports_and_refs` to `true` when only
//! symbol listing is needed (e.g. inspect mode); skips the import and
//! reference query passes, roughly halving per-file extraction time.

use rayon::prelude::*;

use crate::error::{Diagnostic, Severity};
use crate::language::LangId;
use crate::model::{IdGenerator, Symbol, SymbolId};
use crate::parser;

pub use crate::model::FileExtraction;

/// Controls what the extraction pass produces.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExtractOptions {
    /// Skip import and reference extraction entirely; only extract symbols
    /// and AST node counts. Halves per-file extraction cost for pure
    /// symbol-inspection workflows.
    pub skip_imports_and_refs: bool,
}

pub struct ExtractionResult {
    pub files: Vec<FileExtraction>,
}

pub fn extract(files: &[(std::path::PathBuf, LangId)]) -> ExtractionResult {
    extract_with_options(files, &ExtractOptions::default())
}

pub fn extract_with_options(
    files: &[(std::path::PathBuf, LangId)],
    opts: &ExtractOptions,
) -> ExtractionResult {
    let id_gen = IdGenerator::<SymbolId>::new();

    let mut file_extractions: Vec<_> = files
        .par_iter()
        .map(|(path, lang)| extract_single_file(path, lang, &id_gen, opts))
        .collect();

    file_extractions.sort_by(|a, b| a.path.cmp(&b.path));

    ExtractionResult {
        files: file_extractions,
    }
}

fn extract_single_file(
    path: &std::path::Path,
    lang: &LangId,
    id_gen: &IdGenerator<SymbolId>,
    opts: &ExtractOptions,
) -> FileExtraction {
    let source = match std::fs::read(path) {
        Ok(s) => s,
        Err(e) => {
            return FileExtraction {
                path: path.to_path_buf(),
                lang: *lang,
                symbols: Vec::new(),
                imports: Vec::new(),
                references: Vec::new(),
                diagnostics: vec![Diagnostic {
                    path: path.to_path_buf(),
                    severity: Severity::Error,
                    message: format!("failed to read file: {e}"),
                    source_range: None,
                }],
                ast_node_count: 0,
                #[cfg(feature = "metacall-deploy")]
                call_sites: Vec::new(),
            };
        }
    };

    let tree = match crate::parser::parse_tree(*lang, &source) {
        Ok(t) => t,
        Err(e) => {
            return FileExtraction {
                path: path.to_path_buf(),
                lang: *lang,
                symbols: Vec::new(),
                imports: Vec::new(),
                references: Vec::new(),
                diagnostics: vec![Diagnostic {
                    path: path.to_path_buf(),
                    severity: Severity::Error,
                    message: e.to_string(),
                    source_range: None,
                }],
                ast_node_count: 0,
                #[cfg(feature = "metacall-deploy")]
                call_sites: Vec::new(),
            };
        }
    };

    let metrics = parser::tree_metrics(&tree, &source);
    let mut diags = Vec::new();

    if metrics.error_ratio > 0.5 {
        diags.push(Diagnostic {
            path: path.to_path_buf(),
            severity: Severity::Warning,
            message: format!(
                "file has {:.0}% parse errors, results may be incomplete",
                metrics.error_ratio * 100.0
            ),
            source_range: None,
        });
    }

    let raw_symbols = crate::language::extract_symbols_for(*lang, &tree, &source);
    let symbols = raw_symbols
        .into_iter()
        .map(|raw| Symbol {
            id: id_gen.next(),
            name: raw.name.into_owned(),
            kind: raw.kind,
            language: *lang,
            file_path: path.to_path_buf(),
            source_range: raw.source_range,
            visibility: raw.visibility,
            signature: raw.signature.map(|s| s.into_owned()),
            docstring: raw.docstring.map(|s| s.into_owned()),
            is_async: raw.is_async,
        })
        .collect();

    let (imports, references) = if opts.skip_imports_and_refs {
        (Vec::new(), Vec::new())
    } else {
        crate::language::extract_imports_and_references_for(*lang, &tree, &source, path)
    };

    #[cfg(feature = "metacall-deploy")]
    let call_sites = crate::deploy::scanner::scan_file(*lang, &tree, &source, path);

    FileExtraction {
        path: path.to_path_buf(),
        lang: *lang,
        symbols,
        imports,
        references,
        diagnostics: diags,
        ast_node_count: metrics.node_count,
        #[cfg(feature = "metacall-deploy")]
        call_sites,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join("meta_ast_test_extractor");
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn write_temp(name: &str, content: &[u8]) -> PathBuf {
        let path = test_dir().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn extract_single_python_file() {
        let path = write_temp("single.py", b"def hello(): pass\n");
        let result = extract(&[(path.clone(), LangId::Python)]);
        assert_eq!(result.files.len(), 1);
        assert!(!result.files[0].symbols.is_empty());
        assert!(result.files[0].diagnostics.is_empty());
        let names: Vec<&str> = result.files[0]
            .symbols
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert!(names.contains(&"hello"));
    }

    #[test]
    fn extract_multiple_files_parallel() {
        let p1 = write_temp("file_a.py", b"def alpha(): pass\n");
        let p2 = write_temp("file_b.py", b"def beta(): pass\ndef gamma(): pass\n");
        let p3 = write_temp("file_c.py", b"class Delta: pass\n");

        let files = vec![
            (p1.clone(), LangId::Python),
            (p2.clone(), LangId::Python),
            (p3.clone(), LangId::Python),
        ];
        let result = extract(&files);
        let all_names: Vec<&str> = result
            .files
            .iter()
            .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(all_names.contains(&"alpha"), "missing alpha: {all_names:?}");
        assert!(all_names.contains(&"beta"), "missing beta: {all_names:?}");
        assert!(all_names.contains(&"gamma"), "missing gamma: {all_names:?}");
        assert!(all_names.contains(&"Delta"), "missing Delta: {all_names:?}");
    }

    #[test]
    fn accumulate_diagnostics_on_malformed() {
        let path = test_dir().join("nonexistent_broken.py");
        let _ = std::fs::remove_file(&path);
        let result = extract(&[(path, LangId::Python)]);
        assert!(!result.files[0].diagnostics.is_empty());
    }

    #[test]
    fn partial_extraction_on_errors() {
        let valid = write_temp("valid_partial.py", b"def works(): pass\n");
        let broken = write_temp(
            "broken_partial.py",
            b"def broken(\n   # missing close paren and colon\n",
        );
        let result = extract(&[(valid.clone(), LangId::Python), (broken, LangId::Python)]);
        let names: Vec<&str> = result
            .files
            .iter()
            .flat_map(|f| f.symbols.iter().map(|s| s.name.as_str()))
            .collect();
        assert!(
            names.contains(&"works"),
            "valid file symbols should be present: {names:?}"
        );
    }

    #[test]
    fn output_deterministic() {
        let path = write_temp("deterministic.py", b"def foo(): pass\ndef bar(): pass\n");
        let files = vec![(path.clone(), LangId::Python)];

        let r1 = extract(&files);
        let r2 = extract(&files);

        let names1: Vec<String> = r1
            .files
            .iter()
            .flat_map(|f| f.symbols.iter().map(|s| s.name.clone()))
            .collect();
        let names2: Vec<String> = r2
            .files
            .iter()
            .flat_map(|f| f.symbols.iter().map(|s| s.name.clone()))
            .collect();
        assert_eq!(names1, names2);
    }

    #[test]
    fn symbols_assigned_ids() {
        let path = write_temp("ids.py", b"def a(): pass\ndef b(): pass\ndef c(): pass\n");
        let result = extract(&[(path, LangId::Python)]);
        let ids: Vec<u32> = result.files[0]
            .symbols
            .iter()
            .map(|s| s.id.to_raw())
            .collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        assert_eq!(ids, sorted_ids, "IDs should be sequential");

        for window in sorted_ids.windows(2) {
            assert_eq!(window[1] - window[0], 1, "IDs should be consecutive");
        }

        let unique: std::collections::HashSet<u32> = ids.iter().copied().collect();
        assert_eq!(
            unique.len(),
            result.files[0].symbols.len(),
            "all IDs must be unique"
        );
    }
}
