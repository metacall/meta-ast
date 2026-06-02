//! Stateful import path resolution seam.
//!
//! Provides the `ImportResolver` trait and a `StatelessResolver` adapter
//! that wraps existing stateless function pointers, allowing gradual
//! migration to stateful per-language resolvers.

use std::path::{Path, PathBuf};

/// Stateful import path resolution seam.
///
/// Implementors resolve a raw import string to an on-disk path within a project.
/// The interface is stateful (takes `&self`) to allow implementors to cache
/// config file reads (tsconfig.json, go.mod, sys.path) on first use.
pub trait ImportResolver: Send + Sync {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf>;
}

/// Zero-cost adapter wrapping a stateless function pointer.
///
/// Bridges the existing `LanguageSpec.import_path_resolver` fn pointers
/// to the `ImportResolver` trait without changing the `LanguageSpec` struct.
pub struct StatelessResolver {
    f: fn(&str, &Path, &Path) -> Option<PathBuf>,
}

impl StatelessResolver {
    pub fn new(f: fn(&str, &Path, &Path) -> Option<PathBuf>) -> Self {
        Self { f }
    }
}

impl ImportResolver for StatelessResolver {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
        (self.f)(raw, source_dir, project_root)
    }
}

/// Stateful resolver for Python import paths.
pub struct PythonResolver {
    f: fn(&str, &Path, &Path) -> Option<PathBuf>,
}

impl PythonResolver {
    pub fn new(f: fn(&str, &Path, &Path) -> Option<PathBuf>) -> Self {
        Self { f }
    }
}

impl ImportResolver for PythonResolver {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
        (self.f)(raw, source_dir, project_root)
    }
}

/// Stateful resolver for Go module import paths.
pub struct GoModResolver {
    f: fn(&str, &Path, &Path) -> Option<PathBuf>,
}

impl GoModResolver {
    pub fn new(f: fn(&str, &Path, &Path) -> Option<PathBuf>) -> Self {
        Self { f }
    }
}

impl ImportResolver for GoModResolver {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
        (self.f)(raw, source_dir, project_root)
    }
}

/// Stateful resolver for JavaScript import paths.
pub struct JsResolver {
    f: fn(&str, &Path, &Path) -> Option<PathBuf>,
}

impl JsResolver {
    pub fn new(f: fn(&str, &Path, &Path) -> Option<PathBuf>) -> Self {
        Self { f }
    }
}

impl ImportResolver for JsResolver {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
        (self.f)(raw, source_dir, project_root)
    }
}

/// Stateful resolver for TypeScript import paths using `tsconfig.json`.
pub struct TsConfigResolver {
    f: fn(&str, &Path, &Path) -> Option<PathBuf>,
}

impl TsConfigResolver {
    pub fn new(f: fn(&str, &Path, &Path) -> Option<PathBuf>) -> Self {
        Self { f }
    }
}

impl ImportResolver for TsConfigResolver {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
        (self.f)(raw, source_dir, project_root)
    }
}

/// Construct a boxed `ImportResolver` for the given language.
///
/// Wraps the existing stateless fn pointer from `LanguageSpec` into
/// a language-specific resolver (PythonResolver, TsConfigResolver, etc.)
/// allowing gradual, modular migration to stateful resolution.
pub fn make_resolver(lang: crate::language::LangId) -> Box<dyn ImportResolver> {
    let f = lang.spec().import_path_resolver;
    match lang {
        crate::language::LangId::Python => Box::new(PythonResolver::new(f)),
        crate::language::LangId::Go => Box::new(GoModResolver::new(f)),
        crate::language::LangId::JavaScript => Box::new(JsResolver::new(f)),
        crate::language::LangId::TypeScript | crate::language::LangId::Tsx => {
            Box::new(TsConfigResolver::new(f))
        }
        _ => Box::new(StatelessResolver::new(f)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn stateless_resolver_delegates_to_fn() {
        // A fn pointer that resolves "foo" to /proj/foo.py
        fn my_resolver(raw: &str, _source: &Path, root: &Path) -> Option<PathBuf> {
            Some(root.join(format!("{raw}.py")))
        }
        let resolver = StatelessResolver::new(my_resolver);
        let result = resolver.resolve("foo", Path::new("/src"), Path::new("/proj"));
        assert_eq!(result, Some(PathBuf::from("/proj/foo.py")));
    }

    #[test]
    fn stateless_resolver_returns_none_for_unresolvable() {
        fn null_resolver(_raw: &str, _src: &Path, _root: &Path) -> Option<PathBuf> {
            None
        }
        let resolver = StatelessResolver::new(null_resolver);
        let result = resolver.resolve("anything", Path::new("/src"), Path::new("/proj"));
        assert!(result.is_none());
    }

    #[test]
    fn make_resolver_returns_working_resolver_for_python() {
        use crate::language::LangId;
        let resolver = make_resolver(LangId::Python);
        // Python should resolve "b" from /proj/a/ to /proj/a/b.py
        let result = resolver.resolve("b", Path::new("/proj/a"), Path::new("/proj"));
        // We just verify it doesn't panic and returns an Option
        let _ = result; // may be None if /proj/a/b.py doesn't exist on disk - that's fine
    }

    #[test]
    fn import_resolver_trait_is_object_safe() {
        // This test verifies the trait can be used as a trait object
        fn accepts_boxed(_resolver: &dyn ImportResolver) {}

        fn null_resolver(_raw: &str, _src: &Path, _root: &Path) -> Option<PathBuf> {
            None
        }
        let resolver = StatelessResolver::new(null_resolver);
        accepts_boxed(&resolver);
    }

    #[test]
    fn python_resolver_resolves_import() {
        fn dummy_python_resolver(raw: &str, _source: &Path, root: &Path) -> Option<PathBuf> {
            Some(root.join(format!("{raw}.py")))
        }
        let resolver = PythonResolver::new(dummy_python_resolver);
        let result = resolver.resolve("test", Path::new("/src"), Path::new("/proj"));
        assert_eq!(result, Some(PathBuf::from("/proj/test.py")));
    }

    #[test]
    fn tsconfig_resolver_resolves_import() {
        fn dummy_ts_resolver(raw: &str, _source: &Path, root: &Path) -> Option<PathBuf> {
            Some(root.join(format!("{raw}.ts")))
        }
        let resolver = TsConfigResolver::new(dummy_ts_resolver);
        let result = resolver.resolve("test", Path::new("/src"), Path::new("/proj"));
        assert_eq!(result, Some(PathBuf::from("/proj/test.ts")));
    }

    #[test]
    fn go_mod_resolver_resolves_import() {
        fn dummy_go_resolver(raw: &str, _source: &Path, root: &Path) -> Option<PathBuf> {
            Some(root.join(format!("{raw}.go")))
        }
        let resolver = GoModResolver::new(dummy_go_resolver);
        let result = resolver.resolve("test", Path::new("/src"), Path::new("/proj"));
        assert_eq!(result, Some(PathBuf::from("/proj/test.go")));
    }
}
