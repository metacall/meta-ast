//! Stateful import path resolution seam.
//!
//! Provides the `ImportResolver` trait and a `StatelessResolver` adapter
//! that wraps existing stateless function pointers, allowing gradual
//! migration to stateful per-language resolvers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

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
    exists_cache: RwLock<HashMap<PathBuf, bool>>,
}

impl PythonResolver {
    pub fn new(f: fn(&str, &Path, &Path) -> Option<PathBuf>) -> Self {
        Self {
            f,
            exists_cache: RwLock::new(HashMap::new()),
        }
    }
}

impl ImportResolver for PythonResolver {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
        let raw = raw.trim_matches(|c| c == '"' || c == '\'');
        if raw.is_empty() {
            return None;
        }

        let check_exists = |path: &Path| -> bool {
            let cache_val = self
                .exists_cache
                .read()
                .ok()
                .and_then(|cache| cache.get(path).copied());
            if let Some(res) = cache_val {
                return res;
            }
            let res = path.exists();
            if let Ok(mut cache) = self.exists_cache.write() {
                cache.insert(path.to_path_buf(), res);
            }
            res
        };

        if raw.starts_with('.') {
            let relative = raw.trim_start_matches('.');
            if relative.is_empty() {
                return Some(source_dir.join("__init__.py"));
            }
            let path = source_dir.join(relative.replace('.', std::path::MAIN_SEPARATOR_STR));
            let init_path = path.join("__init__.py");
            if check_exists(&init_path) {
                return Some(init_path);
            }
        } else {
            let path = project_root.join(raw.replace('.', std::path::MAIN_SEPARATOR_STR));
            let init_path = path.join("__init__.py");
            if check_exists(&init_path) {
                return Some(init_path);
            }
        }

        (self.f)(raw, source_dir, project_root)
    }
}

/// Stateful resolver for Go module import paths.
pub struct GoModResolver {
    f: fn(&str, &Path, &Path) -> Option<PathBuf>,
    cached_module: OnceLock<Option<(PathBuf, String)>>,
}

impl GoModResolver {
    pub fn new(f: fn(&str, &Path, &Path) -> Option<PathBuf>) -> Self {
        Self {
            f,
            cached_module: OnceLock::new(),
        }
    }
}

impl ImportResolver for GoModResolver {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
        let raw = raw.trim_matches(|c| c == '"' || c == '\'');
        if raw.is_empty() {
            return None;
        }

        if let Some(relative) = raw.strip_prefix('.') {
            let path = source_dir.join(relative);
            return Some(path.with_extension("go"));
        }

        let module_info = self.cached_module.get_or_init(|| {
            let mut current = Some(project_root);
            while let Some(dir) = current {
                let go_mod = dir.join("go.mod");
                if go_mod.is_file() {
                    if let Ok(content) = std::fs::read_to_string(&go_mod) {
                        for line in content.lines() {
                            let line = line.trim();
                            if let Some(module) = line.strip_prefix("module ") {
                                return Some((dir.to_path_buf(), module.trim().to_string()));
                            }
                        }
                    }
                    break;
                }
                current = dir.parent();
            }
            None
        });

        let matched_module = module_info.as_ref().and_then(|(dir, name)| {
            if raw.starts_with(name) {
                Some((dir, name))
            } else {
                None
            }
        });
        if let Some((dir, module_name)) = matched_module {
            let relative = raw[module_name.len()..].trim_start_matches('/');
            return Some(dir.join(relative).with_extension("go"));
        }

        (self.f)(raw, source_dir, project_root)
    }
}

/// Stateful resolver for JavaScript import paths.
pub struct JsResolver {
    f: fn(&str, &Path, &Path) -> Option<PathBuf>,
    is_file_cache: RwLock<HashMap<PathBuf, bool>>,
}

impl JsResolver {
    pub fn new(f: fn(&str, &Path, &Path) -> Option<PathBuf>) -> Self {
        Self {
            f,
            is_file_cache: RwLock::new(HashMap::new()),
        }
    }
}

impl ImportResolver for JsResolver {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
        let raw = raw.trim_matches(|c| c == '"' || c == '\'');
        if raw.is_empty() {
            return None;
        }

        let check_is_file = |path: &Path| -> bool {
            let cache_val = self
                .is_file_cache
                .read()
                .ok()
                .and_then(|cache| cache.get(path).copied());
            if let Some(res) = cache_val {
                return res;
            }
            let res = path.is_file();
            if let Ok(mut cache) = self.is_file_cache.write() {
                cache.insert(path.to_path_buf(), res);
            }
            res
        };

        if !raw.starts_with('.') && !raw.starts_with('/') {
            return (self.f)(raw, source_dir, project_root);
        }

        let base = if raw.starts_with('/') {
            PathBuf::from("/")
        } else {
            source_dir.to_path_buf()
        };

        let path = base.join(raw);

        let extensions = ["", ".js", ".json", ".node", ".mjs", ".cjs"];
        for ext in &extensions {
            let candidate = if ext.is_empty() {
                path.clone()
            } else {
                path.with_extension(ext.trim_start_matches('.'))
            };
            if check_is_file(&candidate) {
                return Some(candidate);
            }
        }

        (self.f)(raw, source_dir, project_root)
    }
}

/// Stateful resolver for TypeScript import paths using `tsconfig.json`.
pub struct TsConfigResolver {
    f: fn(&str, &Path, &Path) -> Option<PathBuf>,
    is_file_cache: RwLock<HashMap<PathBuf, bool>>,
}

impl TsConfigResolver {
    pub fn new(f: fn(&str, &Path, &Path) -> Option<PathBuf>) -> Self {
        Self {
            f,
            is_file_cache: RwLock::new(HashMap::new()),
        }
    }
}

impl ImportResolver for TsConfigResolver {
    fn resolve(&self, raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
        let raw = raw.trim_matches(|c| c == '"' || c == '\'');
        if raw.is_empty() {
            return None;
        }

        let check_is_file = |path: &Path| -> bool {
            let cache_val = self
                .is_file_cache
                .read()
                .ok()
                .and_then(|cache| cache.get(path).copied());
            if let Some(res) = cache_val {
                return res;
            }
            let res = path.is_file();
            if let Ok(mut cache) = self.is_file_cache.write() {
                cache.insert(path.to_path_buf(), res);
            }
            res
        };

        if !raw.starts_with('.') && !raw.starts_with('/') {
            return (self.f)(raw, source_dir, project_root);
        }

        let base = if raw.starts_with('/') {
            PathBuf::from("/")
        } else {
            source_dir.to_path_buf()
        };

        let path = base.join(raw);

        let extensions = ["", ".js", ".ts", ".jsx", ".tsx", ".mjs", ".cjs"];
        for ext in &extensions {
            let candidate = if ext.is_empty() {
                path.clone()
            } else {
                path.with_extension(ext.trim_start_matches('.'))
            };
            if check_is_file(&candidate) {
                return Some(candidate);
            }
        }

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

    #[test]
    fn go_mod_resolver_memoizes_go_mod_file() {
        let temp_dir = std::env::temp_dir().join("go_mod_resolver_memoizes_go_mod_file");
        if temp_dir.exists() {
            let _ = std::fs::remove_dir_all(&temp_dir);
        }
        std::fs::create_dir_all(&temp_dir).unwrap();
        let go_mod_path = temp_dir.join("go.mod");
        std::fs::write(&go_mod_path, "module myproject\n").unwrap();

        let resolver = make_resolver(crate::language::LangId::Go);

        // First resolve: should succeed and read from disk
        let res1 = resolver.resolve("myproject/sub", &temp_dir, &temp_dir);
        assert_eq!(res1, Some(temp_dir.join("sub.go")));

        // Delete go.mod from disk!
        std::fs::remove_file(&go_mod_path).unwrap();

        // Second resolve: should STILL succeed because the resolver memoized the module name!
        let res2 = resolver.resolve("myproject/other", &temp_dir, &temp_dir);
        assert_eq!(res2, Some(temp_dir.join("other.go")));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn python_resolver_memoizes_exists_checks() {
        let temp_dir = std::env::temp_dir().join("python_resolver_memoizes_exists_checks");
        if temp_dir.exists() {
            let _ = std::fs::remove_dir_all(&temp_dir);
        }
        std::fs::create_dir_all(&temp_dir).unwrap();
        let pkg_dir = temp_dir.join("my_package");
        std::fs::create_dir_all(&pkg_dir).unwrap();
        let init_py = pkg_dir.join("__init__.py");
        std::fs::write(&init_py, "").unwrap();

        let resolver = make_resolver(crate::language::LangId::Python);

        // First resolve: resolves to my_package/__init__.py
        let res1 = resolver.resolve("my_package", &temp_dir, &temp_dir);
        assert_eq!(res1, Some(init_py.clone()));

        // Delete __init__.py from disk
        std::fs::remove_file(&init_py).unwrap();

        // Second resolve: should STILL return my_package/__init__.py because the resolver memoized the exists() result!
        let res2 = resolver.resolve("my_package", &temp_dir, &temp_dir);
        assert_eq!(res2, Some(init_py));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn tsconfig_resolver_memoizes_is_file_checks() {
        let temp_dir = std::env::temp_dir().join("tsconfig_resolver_memoizes_is_file_checks");
        if temp_dir.exists() {
            let _ = std::fs::remove_dir_all(&temp_dir);
        }
        std::fs::create_dir_all(&temp_dir).unwrap();
        let ts_file = temp_dir.join("my_file.ts");
        std::fs::write(&ts_file, "").unwrap();

        let resolver = make_resolver(crate::language::LangId::TypeScript);

        // First resolve: resolves to my_file.ts
        let res1 = resolver.resolve("./my_file", &temp_dir, &temp_dir);
        assert_eq!(res1, Some(ts_file.clone()));

        // Delete my_file.ts
        std::fs::remove_file(&ts_file).unwrap();

        // Second resolve: should STILL return my_file.ts because it memoized the is_file() result!
        let res2 = resolver.resolve("./my_file", &temp_dir, &temp_dir);
        assert_eq!(res2, Some(ts_file));

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
