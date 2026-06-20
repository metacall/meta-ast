use crate::language::LangId;

/// Maps meta-ast's LangId to MetaCall runtime tags.
pub fn metacall_tag(lang: LangId) -> &'static str {
    match lang {
        LangId::Python => "py",
        LangId::JavaScript => "node",
        LangId::TypeScript | LangId::Tsx => "ts",
        LangId::C => "c",
        LangId::Cpp => "cpp",
        LangId::Rust => "rs",
        LangId::Go => "go",
    }
}

/// Maps MetaCall runtime tags back to meta-ast's LangId.
pub fn from_metacall_tag(tag: &str) -> Option<LangId> {
    match tag {
        "py" => Some(LangId::Python),
        "node" => Some(LangId::JavaScript),
        "ts" => Some(LangId::TypeScript),
        "c" => Some(LangId::C),
        "cpp" => Some(LangId::Cpp),
        "rs" => Some(LangId::Rust),
        "go" => Some(LangId::Go),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metacall_tag_mapping() {
        assert_eq!(metacall_tag(LangId::Python), "py");
        assert_eq!(metacall_tag(LangId::JavaScript), "node");
        assert_eq!(metacall_tag(LangId::TypeScript), "ts");
        assert_eq!(metacall_tag(LangId::Tsx), "ts");
        assert_eq!(metacall_tag(LangId::C), "c");
        assert_eq!(metacall_tag(LangId::Cpp), "cpp");
        assert_eq!(metacall_tag(LangId::Rust), "rs");
        assert_eq!(metacall_tag(LangId::Go), "go");
    }
}
