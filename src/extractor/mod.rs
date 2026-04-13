use rayon::prelude::*;

use crate::error::{Diagnostic, Error, Severity};
use crate::language::LangId;
use crate::model::{IdGenerator, Symbol, SymbolId};
use crate::parser;

pub struct ExtractionResult {
    pub symbols: Vec<Symbol>,
    pub diagnostics: Vec<Diagnostic>,
}

pub fn extract(files: &[(std::path::PathBuf, LangId)]) -> ExtractionResult {
    let id_gen = IdGenerator::<SymbolId>::new();
    let mut diagnostics = Vec::new();

    let results: Vec<_> = files
        .par_iter()
        .filter_map(
            |(path, lang)| match extract_single_file(path, lang, &id_gen) {
                Ok((symbols, diags)) => Some((path.clone(), *lang, symbols, diags)),
                Err(e) => {
                    let diag = Diagnostic {
                        path: path.clone(),
                        severity: Severity::Error,
                        message: e.to_string(),
                        source_range: None,
                    };
                    Some((path.clone(), *lang, Vec::new(), vec![diag]))
                }
            },
        )
        .collect();

    let mut symbols = Vec::new();
    for (_path, _lang, mut file_symbols, mut diags) in results {
        diagnostics.append(&mut diags);
        symbols.append(&mut file_symbols);
    }

    symbols.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then(a.source_range.byte_start.cmp(&b.source_range.byte_start))
    });

    ExtractionResult {
        symbols,
        diagnostics,
    }
}

fn extract_single_file(
    path: &std::path::Path,
    lang: &LangId,
    id_gen: &IdGenerator<SymbolId>,
) -> Result<(Vec<Symbol>, Vec<Diagnostic>), Error> {
    let source = std::fs::read(path)?;
    let tree = parser::parse_tree(*lang, &source)?;

    let ratio = parser::error_ratio(&tree, &source);
    let mut diags = Vec::new();

    if ratio > 0.5 {
        diags.push(Diagnostic {
            path: path.to_path_buf(),
            severity: Severity::Warning,
            message: format!(
                "file has {:.0}% parse errors, results may be incomplete",
                ratio * 100.0
            ),
            source_range: None,
        });
    }

    let mut cursor = tree.walk();
    let raw_symbols = crate::language::extract_symbols_for(*lang, &tree, &source, &mut cursor);

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

    Ok((symbols, diags))
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
        assert!(!result.symbols.is_empty());
        assert!(result.diagnostics.is_empty());
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
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
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"alpha"), "missing alpha: {names:?}");
        assert!(names.contains(&"beta"), "missing beta: {names:?}");
        assert!(names.contains(&"gamma"), "missing gamma: {names:?}");
        assert!(names.contains(&"Delta"), "missing Delta: {names:?}");
    }

    #[test]
    fn accumulate_diagnostics_on_malformed() {
        let path = test_dir().join("nonexistent_broken.py");
        let _ = std::fs::remove_file(&path);
        let result = extract(&[(path, LangId::Python)]);
        assert!(!result.diagnostics.is_empty());
    }

    #[test]
    fn partial_extraction_on_errors() {
        let valid = write_temp("valid_partial.py", b"def works(): pass\n");
        let broken = write_temp(
            "broken_partial.py",
            b"def broken(\n   # missing close paren and colon\n",
        );
        let result = extract(&[(valid.clone(), LangId::Python), (broken, LangId::Python)]);
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
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

        let names1: Vec<String> = r1.symbols.iter().map(|s| s.name.clone()).collect();
        let names2: Vec<String> = r2.symbols.iter().map(|s| s.name.clone()).collect();
        assert_eq!(names1, names2);
    }

    #[test]
    fn symbols_assigned_ids() {
        let path = write_temp("ids.py", b"def a(): pass\ndef b(): pass\ndef c(): pass\n");
        let result = extract(&[(path, LangId::Python)]);
        assert!(!result.symbols.is_empty());

        let ids: Vec<u32> = result.symbols.iter().map(|s| s.id.to_raw()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        assert_eq!(ids, sorted_ids, "IDs should be sequential");

        for window in sorted_ids.windows(2) {
            assert_eq!(window[1] - window[0], 1, "IDs should be consecutive");
        }

        let unique: std::collections::HashSet<u32> = ids.iter().copied().collect();
        assert_eq!(unique.len(), result.symbols.len(), "all IDs must be unique");
    }
}
