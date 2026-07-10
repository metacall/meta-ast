use crate::language::LangId;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallSiteVariant {
    LoadFromFile,
    LoadFromMemory,
    LoadFromPackage,
    LoadFromConfiguration,
}

#[derive(Debug, Clone)]
pub struct CallSite {
    pub source_file: PathBuf,
    pub caller_lang: LangId,
    pub variant: CallSiteVariant,
    pub target_lang: Option<String>,
    pub scripts: Vec<String>,
    pub confidence: f64,
}

impl CallSiteVariant {
    fn from_str(s: &str) -> Option<Self> {
        if s.contains("load_from_file") || s.contains("LoadFromFile") {
            Some(Self::LoadFromFile)
        } else if s.contains("load_from_memory") || s.contains("LoadFromMemory") {
            Some(Self::LoadFromMemory)
        } else if s.contains("load_from_package") || s.contains("LoadFromPackage") {
            Some(Self::LoadFromPackage)
        } else if s.contains("load_from_configuration") || s.contains("LoadFromConfiguration") {
            Some(Self::LoadFromConfiguration)
        } else if s.contains("from_file") {
            Some(Self::LoadFromFile)
        } else if s.contains("from_memory") {
            Some(Self::LoadFromMemory)
        } else if s.contains("from_package") {
            Some(Self::LoadFromPackage)
        } else if s.contains("from_configuration") {
            Some(Self::LoadFromConfiguration)
        } else {
            None
        }
    }
}

fn strip_quotes(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

fn get_node_text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    std::str::from_utf8(&source[node.byte_range()]).unwrap_or("")
}

fn collect_strings_recursive(node: Node, source: &[u8], scripts: &mut Vec<String>) {
    let kind = node.kind();
    if kind.contains("string") || kind == "string_literal" {
        scripts.push(strip_quotes(get_node_text(node, source)));
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                collect_strings_recursive(child, source, scripts);
            }
        }
    }
}

static PYTHON_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_python::LANGUAGE.into(),
        r#"
(call
  function: (identifier) @fn_name
  arguments: (argument_list) @args
  (#match? @fn_name "^metacall_load_from_"))
"#,
        "Python deploy",
    )
});

static JS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_javascript::LANGUAGE.into(),
        r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args
  (#match? @fn_name "^metacall_load_from_"))
"#,
        "JS deploy",
    )
});

static TS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args
  (#match? @fn_name "^metacall_load_from_"))
"#,
        "TS deploy",
    )
});

static TSX_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args
  (#match? @fn_name "^metacall_load_from_"))
"#,
        "TSX deploy",
    )
});

static C_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_c::LANGUAGE.into(),
        r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (argument_list) @args
  (#match? @fn_name "^metacall_load_from_"))
"#,
        "C deploy",
    )
});

static CPP_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_cpp::LANGUAGE.into(),
        r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (argument_list) @args
  (#match? @fn_name "^metacall_load_from_"))
"#,
        "CPP deploy",
    )
});

static RUST_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_rust::LANGUAGE.into(),
        r#"
(call_expression
  function: [
    (scoped_identifier
        path: (identifier) @mod_name
        name: (identifier) @fn_name)
    (scoped_identifier
        path: (scoped_identifier path: (identifier) @mod_name name: (identifier) @sub_mod)
        name: (identifier) @fn_name)
  ]
  arguments: (arguments) @args
  (#match? @mod_name "metacall"))
"#,
        "Rust deploy",
    )
});

static GO_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_go::LANGUAGE.into(),
        r#"
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg_name
    field: (field_identifier) @fn_name)
  arguments: (argument_list) @args
  (#match? @pkg_name "metacall")
  (#match? @fn_name "LoadFrom"))
"#,
        "Go deploy",
    )
});

pub fn scan_file(id: LangId, tree: &Tree, source: &[u8], path: &Path) -> Vec<CallSite> {
    let query = match id {
        LangId::Python => &*PYTHON_QUERY,
        LangId::JavaScript => &*JS_QUERY,
        LangId::TypeScript => &*TS_QUERY,
        LangId::Tsx => &*TSX_QUERY,
        LangId::C => &*C_QUERY,
        LangId::Cpp => &*CPP_QUERY,
        LangId::Rust => &*RUST_QUERY,
        LangId::Go => &*GO_QUERY,
    };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    let mut call_sites = Vec::new();

    // Capture indices are static query-shape facts; a missing name means a
    // malformed query constant, not runtime data. Bail out rather than panic.
    let Some(fn_name_idx) = query.capture_index_for_name("fn_name") else {
        return call_sites;
    };
    let Some(args_idx) = query.capture_index_for_name("args") else {
        return call_sites;
    };

    while let Some(mat) = matches.next() {
        let mut variant = None;
        let mut target_lang = None;
        let mut scripts = Vec::new();
        let mut confidence = 1.0;

        let mut args_node = None;

        for capture in mat.captures {
            if capture.index == fn_name_idx {
                let name = get_node_text(capture.node, source);
                variant = CallSiteVariant::from_str(name);
            } else if capture.index == args_idx {
                args_node = Some(capture.node);
            }
        }

        if let (Some(variant), Some(args)) = (variant, args_node) {
            // Process arguments
            let mut named_children = Vec::new();
            let mut cursor = args.walk();
            for child in args.children(&mut cursor) {
                if child.is_named() {
                    named_children.push(child);
                }
            }

            if let Some(lang_node) = named_children.first() {
                let text = get_node_text(*lang_node, source);
                let kind = lang_node.kind();
                if kind.contains("string") || kind == "string_literal" {
                    target_lang = Some(strip_quotes(text));
                } else {
                    target_lang = Some(text.to_string());
                    confidence = 0.4;
                }
            }

            if let Some(scripts_node) = named_children.get(1) {
                let kind = scripts_node.kind();
                if kind == "list"
                    || kind == "array"
                    || kind == "array_expression"
                    || kind == "literal_value"
                    || kind == "composite_literal"
                {
                    collect_strings_recursive(*scripts_node, source, &mut scripts);
                } else {
                    let text = get_node_text(*scripts_node, source);
                    if kind.contains("string") || kind == "string_literal" {
                        scripts.push(strip_quotes(text));
                    } else {
                        scripts.push(text.to_string());
                        confidence = 0.4;
                    }
                }
            }

            call_sites.push(CallSite {
                source_file: path.to_path_buf(),
                caller_lang: id,
                variant,
                target_lang,
                scripts,
                confidence,
            });
        }
    }

    call_sites
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::grammar_for;

    fn parse(id: LangId, source: &[u8]) -> Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(id)).unwrap();
        parser.parse(source, None).unwrap()
    }

    #[test]
    fn test_scan_python() {
        let source = b"metacall_load_from_file('node', ['sum.js'])";
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
        assert_eq!(sites[0].scripts, vec!["sum.js"]);
        assert_eq!(sites[0].confidence, 1.0);
    }

    #[test]
    fn test_scan_javascript() {
        let source = b"metacall_load_from_file('py', ['sum.py'])";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_file(LangId::JavaScript, &tree, source, Path::new("test.js"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("py"));
        assert_eq!(sites[0].scripts, vec!["sum.py"]);
    }

    #[test]
    fn test_scan_rust() {
        let source = b"metacall::load_from_file(\"py\", [\"sum.py\"])";
        let tree = parse(LangId::Rust, source);
        let sites = scan_file(LangId::Rust, &tree, source, Path::new("lib.rs"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("py"));
        assert_eq!(sites[0].scripts, vec!["sum.py"]);
    }

    #[test]
    fn test_scan_computed_args() {
        let source = b"metacall_load_from_file(LANG, ['sum.js'])";
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].confidence, 0.4);
        assert_eq!(sites[0].target_lang.as_deref(), Some("LANG"));
    }

    #[test]
    fn test_scan_go() {
        let source = b"metacall.LoadFromFile(\"py\", []string{\"sum.py\"})";
        let tree = parse(LangId::Go, source);
        let sites = scan_file(LangId::Go, &tree, source, Path::new("main.go"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("py"));
        assert_eq!(sites[0].scripts, vec!["sum.py"]);
    }
}
