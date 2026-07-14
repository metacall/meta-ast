//! File discovery and language detection.
//!
//! Walks a directory tree, maps file extensions to `LangId` via a
//! pre-computed extension map, and returns sorted file lists.

use std::path::Path;

use crate::language::{LangId, spec_for};

use std::collections::HashMap;
use std::sync::LazyLock;

static EXT_MAP: LazyLock<HashMap<&'static str, LangId>> = LazyLock::new(|| {
    LangId::all()
        .iter()
        .flat_map(|&id| {
            let spec = spec_for(id);
            spec.extensions.iter().map(move |&ext| (ext, id))
        })
        .collect()
});

pub fn detect_language(path: &Path) -> Option<LangId> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    EXT_MAP.get(ext.as_str()).copied()
}

pub fn discover_files(
    root: &Path,
    languages: Option<&[LangId]>,
) -> Result<Vec<(std::path::PathBuf, LangId)>, std::io::Error> {
    let mut results = Vec::new();

    if root.is_file() {
        if let Some(lang_id) = detect_language(root) {
            results.push((root.to_path_buf(), lang_id));
        }
        return Ok(results);
    }

    if !root.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("path does not exist: {}", root.display()),
        ));
    }

    for entry in ignore::WalkBuilder::new(root)
        .build()
        .filter_map(|e| e.ok())
    {
        let path = entry.into_path();
        if !path.is_file() {
            continue;
        }
        if let Some(lang_id) = detect_language(&path) {
            results.push((path, lang_id));
        }
    }

    if let Some(langs) = languages {
        results.retain(|(_, id)| langs.contains(id));
    }

    results.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(results)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::language::LangId;

    #[test]
    fn detect_python_extensions() {
        assert_eq!(
            detect_language(&PathBuf::from("foo.py")),
            Some(LangId::Python)
        );
        assert_eq!(
            detect_language(&PathBuf::from("foo.pyi")),
            Some(LangId::Python)
        );
    }

    #[test]
    fn detect_javascript_extensions() {
        assert_eq!(
            detect_language(&PathBuf::from("foo.js")),
            Some(LangId::JavaScript)
        );
        assert_eq!(
            detect_language(&PathBuf::from("foo.mjs")),
            Some(LangId::JavaScript)
        );
        assert_eq!(
            detect_language(&PathBuf::from("foo.cjs")),
            Some(LangId::JavaScript)
        );
    }

    #[test]
    fn detect_typescript_extensions() {
        assert_eq!(
            detect_language(&PathBuf::from("foo.ts")),
            Some(LangId::TypeScript)
        );
        assert_eq!(
            detect_language(&PathBuf::from("foo.cts")),
            Some(LangId::TypeScript)
        );
        assert_eq!(
            detect_language(&PathBuf::from("foo.mts")),
            Some(LangId::TypeScript)
        );
    }

    #[test]
    fn detect_tsx_extension() {
        assert_eq!(
            detect_language(&PathBuf::from("foo.tsx")),
            Some(LangId::Tsx)
        );
        assert_ne!(
            detect_language(&PathBuf::from("foo.tsx")),
            Some(LangId::TypeScript)
        );
    }

    #[test]
    fn detect_c_cpp_extensions() {
        assert_eq!(detect_language(&PathBuf::from("foo.c")), Some(LangId::C));
        assert_eq!(detect_language(&PathBuf::from("foo.cc")), Some(LangId::Cpp));
        assert_eq!(
            detect_language(&PathBuf::from("foo.cpp")),
            Some(LangId::Cpp)
        );
        assert_eq!(
            detect_language(&PathBuf::from("foo.cxx")),
            Some(LangId::Cpp)
        );
    }

    #[test]
    fn detect_rust_go() {
        assert_eq!(
            detect_language(&PathBuf::from("foo.rs")),
            Some(LangId::Rust)
        );
        assert_eq!(detect_language(&PathBuf::from("foo.go")), Some(LangId::Go));
    }

    #[test]
    fn detect_unknown_returns_none() {
        assert_eq!(detect_language(&PathBuf::from("foo.md")), None);
        assert_eq!(detect_language(&PathBuf::from("foo.txt")), None);
        assert_eq!(detect_language(&PathBuf::from("README")), None);
    }

    #[test]
    fn detect_case_insensitive() {
        assert_eq!(
            detect_language(&PathBuf::from("foo.PY")),
            Some(LangId::Python)
        );
        assert_eq!(
            detect_language(&PathBuf::from("foo.Rs")),
            Some(LangId::Rust)
        );
    }

    #[test]
    fn discover_files_finds_fixtures() {
        let root = PathBuf::from("tests/fixtures/python");
        let files = discover_files(&root, None).unwrap();
        assert!(!files.is_empty());
        assert!(files.iter().all(|(_, lang)| *lang == LangId::Python));
    }

    #[test]
    fn discover_files_filters_by_language() {
        let root = PathBuf::from("tests/fixtures/mixed");
        let files = discover_files(&root, Some(&[LangId::Python])).unwrap();
        assert!(files.iter().all(|(_, lang)| *lang == LangId::Python));
        assert!(!files.is_empty());
    }

    #[test]
    fn discover_files_empty_dir() {
        let tmp = std::env::temp_dir().join("meta_ast_test_empty");
        std::fs::create_dir_all(&tmp).unwrap();
        let files = discover_files(&tmp, None).unwrap();
        assert!(files.is_empty());
        std::fs::remove_dir(&tmp).unwrap();
    }

    #[test]
    fn discover_single_file() {
        let path = PathBuf::from("tests/fixtures/python/simple_functions.py");
        let files = discover_files(&path, None).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].1, LangId::Python);
    }

    #[test]
    fn discover_files_respects_gitignore() {
        let root = PathBuf::from("tests/fixtures/mixed");
        let files = discover_files(&root, None).unwrap();
        let paths: Vec<_> = files
            .iter()
            .map(|(p, _)| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(
            !paths.contains(&"test.generated.py"),
            "gitignored file should be excluded"
        );
    }

    #[test]
    fn discover_files_sorted() {
        let root = PathBuf::from("tests/fixtures/mixed");
        let files = discover_files(&root, None).unwrap();
        let paths: Vec<_> = files.iter().map(|(p, _)| p).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn ext_map_covers_all_catalog_extensions() {
        for id in LangId::all() {
            let spec = spec_for(id);
            for &ext in spec.extensions {
                assert_eq!(
                    detect_language(&PathBuf::from(format!("foo.{ext}"))),
                    Some(id),
                    "extension {ext:?} should map to {id:?}"
                );
            }
        }
    }
}
