use crate::language::LangId;

/// Canonical MetaCall loader tag for a language.
///
/// C and C++ share the `c` loader (libclang). Go has no loader yet; the tag
/// stays stable for manifests and [`has_loader`] reports the gap.
pub fn metacall_tag(lang: LangId) -> &'static str {
    match lang {
        LangId::Python => "py",
        LangId::JavaScript => "node",
        LangId::TypeScript | LangId::Tsx => "ts",
        LangId::C | LangId::Cpp => "c",
        LangId::Rust => "rs",
        LangId::Go => "go",
        LangId::Ruby => "rb",
    }
}

/// Reports whether MetaCall ships a loader for this language.
pub fn has_loader(lang: LangId) -> bool {
    !matches!(lang, LangId::Go)
}

/// Resolves a MetaCall tag or port alias to a language.
///
/// The mapping is not symmetric for C++: [`metacall_tag`] maps `Cpp` to
/// `"c"`, and `"c"` resolves to `C`. Use the `"cpp"` alias to recover `Cpp`.
pub fn from_metacall_tag(tag: &str) -> Option<LangId> {
    let tag = tag.trim().to_ascii_lowercase();
    match tag.as_str() {
        "py" | "python" => Some(LangId::Python),
        "node" | "nodejs" | "js" | "mjs" | "cjs" => Some(LangId::JavaScript),
        "ts" | "typescript" | "tsx" | "jsx" => Some(LangId::TypeScript),
        "c" => Some(LangId::C),
        "cpp" | "cxx" | "cc" | "c++" => Some(LangId::Cpp),
        "rs" | "rust" => Some(LangId::Rust),
        "go" | "golang" => Some(LangId::Go),
        "rb" | "ruby" => Some(LangId::Ruby),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_mapping_is_canonical() {
        assert_eq!(metacall_tag(LangId::Python), "py");
        assert_eq!(metacall_tag(LangId::JavaScript), "node");
        assert_eq!(metacall_tag(LangId::TypeScript), "ts");
        assert_eq!(metacall_tag(LangId::Tsx), "ts");
        assert_eq!(metacall_tag(LangId::C), "c");
        assert_eq!(metacall_tag(LangId::Cpp), "c");
        assert_eq!(metacall_tag(LangId::Rust), "rs");
        assert_eq!(metacall_tag(LangId::Go), "go");
        assert_eq!(metacall_tag(LangId::Ruby), "rb");
    }

    #[test]
    fn aliases_resolve() {
        let cases = [
            ("python", LangId::Python),
            ("PY", LangId::Python),
            ("nodejs", LangId::JavaScript),
            ("js", LangId::JavaScript),
            ("mjs", LangId::JavaScript),
            ("cjs", LangId::JavaScript),
            ("typescript", LangId::TypeScript),
            ("tsx", LangId::TypeScript),
            ("jsx", LangId::TypeScript),
            ("c++", LangId::Cpp),
            ("cxx", LangId::Cpp),
            ("cc", LangId::Cpp),
            ("rust", LangId::Rust),
            ("ruby", LangId::Ruby),
            ("golang", LangId::Go),
            (" Node ", LangId::JavaScript),
        ];
        for (tag, expected) in cases {
            assert_eq!(from_metacall_tag(tag), Some(expected), "tag {tag}");
        }
    }

    #[test]
    fn cpp_shares_the_c_loader() {
        assert_eq!(metacall_tag(LangId::Cpp), "c");
        assert_eq!(from_metacall_tag("c"), Some(LangId::C));
        assert_eq!(from_metacall_tag("cpp"), Some(LangId::Cpp));
    }

    #[test]
    fn go_has_no_loader() {
        assert_eq!(metacall_tag(LangId::Go), "go");
        assert!(!has_loader(LangId::Go));
        assert!(has_loader(LangId::Python));
        assert!(has_loader(LangId::Cpp));
    }

    #[test]
    fn unknown_tags_are_rejected() {
        assert_eq!(from_metacall_tag("wasm"), None);
        assert_eq!(from_metacall_tag(""), None);
    }
}
