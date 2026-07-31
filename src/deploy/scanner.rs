//! MetaCall call-site scanner.
//!
//! Emits one `CallSite` per MetaCall load or client call detected by tree-sitter queries.

use crate::language::LangId;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use serde::Serialize;

/// Variant of a MetaCall call site: a load API or a client invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub enum CallSiteVariant {
    LoadFromFile,
    LoadFromMemory,
    LoadFromPackage,
    LoadFromConfiguration,
    ClientCall, // metacall, metacall_await, metacallfms, metacallv, metacallt,
                // metacall_function, metacall::metacall, Go Call/Await
}

#[derive(Debug, Clone, Serialize)]
pub struct CallSite {
    pub source_file: PathBuf,
    pub caller_lang: LangId,
    pub variant: CallSiteVariant,
    pub target_lang: Option<String>,
    pub scripts: Vec<String>,
    /// Invocation target function name (`ClientCall` only).
    pub function_name: Option<String>,
    /// True for `metacall_await` and Go `Await`.
    pub is_async: bool,
    /// Argument range of the call, for diagnostics.
    pub source_range: Option<crate::model::SourceRange>,
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
        } else if s.contains("from_file") || s.contains("from_single_file") {
            Some(Self::LoadFromFile)
        } else if s.contains("from_memory") {
            Some(Self::LoadFromMemory)
        } else if s.contains("from_package") {
            Some(Self::LoadFromPackage)
        } else if s.contains("from_configuration") {
            Some(Self::LoadFromConfiguration)
        } else if matches!(
            s,
            "metacall"
                | "metacall_await"
                | "metacall_no_arg"
                | "metacall_untyped"
                | "metacallfms"
                | "metacallv"
                | "metacallt"
                | "metacall_function"
                | "Call"
                | "Await"
        ) {
            // metacall_handle excluded: its argument layout differs per port
            // (tag first in C/Node, handle first in Rust).
            Some(Self::ClientCall)
        } else {
            None
        }
    }
}

fn is_async_call(name: &str) -> bool {
    name.contains("await") || name.contains("Await")
}

fn strip_quotes(s: &str) -> String {
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .to_string()
}

/// True when the string node is static text; interpolations (f-strings,
/// template substitutions) mean the runtime name is computed.
fn is_plain_string(node: Node) -> bool {
    let kind = node.kind();
    if !(kind.contains("string") || kind == "string_literal") {
        return false;
    }
    let mut cursor = node.walk();
    !node
        .children(&mut cursor)
        .any(|c| c.kind() == "interpolation" || c.kind() == "template_substitution")
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
  (#match? @fn_name "^(metacall_load_from_.*|metacall|metacall_await|metacallfms)$"))
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
  (#match? @fn_name "^(metacall_load_from_.*|metacall|metacall_await|metacallfms)$"))
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
  (#match? @fn_name "^(metacall_load_from_.*|metacall|metacall_await|metacallfms)$"))
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
  (#match? @fn_name "^(metacall_load_from_.*|metacall|metacall_await|metacallfms)$"))
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
  (#match? @fn_name "^(metacall_load_from_.*|metacall|metacall_await|metacallfms|metacallv|metacallt|metacall_function)$"))
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
  (#match? @fn_name "^(metacall_load_from_.*|metacall|metacall_await|metacallfms|metacallv|metacallt|metacall_function)$"))
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
    (identifier) @fn_name
  ]
  arguments: (arguments) @args)
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
  (#match? @fn_name "^(LoadFrom.*|Call|Await)$"))
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
        let mut name = "";

        let mut args_node = None;

        for capture in mat.captures {
            if capture.index == fn_name_idx {
                name = get_node_text(capture.node, source);
                variant = CallSiteVariant::from_str(name);
            } else if capture.index == args_idx {
                args_node = Some(capture.node);
            }
        }

        if let (Some(variant), Some(args)) = (variant, args_node) {
            let is_async = is_async_call(name);
            let call_range = args.range();
            let source_range = Some(crate::model::SourceRange {
                byte_start: call_range.start_byte,
                byte_end: call_range.end_byte,
                start: crate::model::LineColumn {
                    line: call_range.start_point.row,
                    column: call_range.start_point.column,
                },
                end: crate::model::LineColumn {
                    line: call_range.end_point.row,
                    column: call_range.end_point.column,
                },
            });

            // Process arguments
            let mut named_children = Vec::new();
            let mut cursor = args.walk();
            for child in args.children(&mut cursor) {
                if child.is_named() {
                    named_children.push(child);
                }
            }

            let mut function_name = None;

            if variant == CallSiteVariant::ClientCall {
                // First argument is the target function name; computed names
                // keep the source text at 0.4 (existing convention).
                if let Some(fn_node) = named_children.first() {
                    let text = get_node_text(*fn_node, source);
                    if is_plain_string(*fn_node) {
                        function_name = Some(strip_quotes(text));
                    } else {
                        function_name = Some(text.to_string());
                        confidence = 0.4;
                    }
                }
            } else {
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
            }

            call_sites.push(CallSite {
                source_file: path.to_path_buf(),
                caller_lang: id,
                variant,
                target_lang,
                scripts,
                function_name,
                is_async,
                source_range,
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
    fn test_scan_rust_bare_name() {
        // After `use metacall::metacall_load_from_file`, the call is bare.
        let source = b"metacall_load_from_file(\"py\", [\"sum.py\"])";
        let tree = parse(LangId::Rust, source);
        let sites = scan_file(LangId::Rust, &tree, source, Path::new("lib.rs"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("py"));
        assert_eq!(sites[0].scripts, vec!["sum.py"]);
    }

    #[test]
    fn test_scan_python_load_from_memory() {
        let source = b"metacall_load_from_memory('node', 'console.log(\"hi\")')";
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromMemory);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
    }

    #[test]
    fn test_scan_python_load_from_package() {
        let source = b"metacall_load_from_package('node', 'express')";
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromPackage);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
        assert_eq!(sites[0].scripts, vec!["express"]);
    }

    #[test]
    fn test_scan_go_load_from_memory() {
        let source = b"metacall.LoadFromMemory(\"node\", []string{\"const x = 1;\"})";
        let tree = parse(LangId::Go, source);
        let sites = scan_file(LangId::Go, &tree, source, Path::new("main.go"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromMemory);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
    }

    #[test]
    fn test_scan_python_client_call() {
        let source = b"metacall('sum', 1, 2)";
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
        assert!(!sites[0].is_async);
        assert!(sites[0].scripts.is_empty());
        assert_eq!(sites[0].confidence, 1.0);
        assert!(sites[0].source_range.is_some());
    }

    #[test]
    fn test_scan_python_client_await() {
        let source = b"metacall_await('sum', 1)";
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert!(sites[0].is_async);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_python_computed_function_name() {
        let source = b"metacall(fn_name, 1)";
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("fn_name"));
        assert_eq!(sites[0].confidence, 0.4);
    }

    #[test]
    fn test_scan_javascript_client_call() {
        let source = b"metacall('sum', 1, 2)";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_file(LangId::JavaScript, &tree, source, Path::new("test.js"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_c_load_from_file() {
        let source = b"metacall_load_from_file(\"node\", paths, size, &handle);";
        let tree = parse(LangId::C, source);
        let sites = scan_file(LangId::C, &tree, source, Path::new("test.c"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
    }

    #[test]
    fn test_scan_c_client_call() {
        let source = b"metacall(\"sum\", 1, 2);\nmetacallv(\"sum\", args);";
        let tree = parse(LangId::C, source);
        let sites = scan_file(LangId::C, &tree, source, Path::new("test.c"));
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[1].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_go_client_call() {
        let source = b"metacall.Call(\"sum\", 1, 2)";
        let tree = parse(LangId::Go, source);
        let sites = scan_file(LangId::Go, &tree, source, Path::new("main.go"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_go_client_await() {
        let source = b"metacall.Await(\"sum\", resolve, reject, ctx, 1)";
        let tree = parse(LangId::Go, source);
        let sites = scan_file(LangId::Go, &tree, source, Path::new("main.go"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert!(sites[0].is_async);
    }

    #[test]
    fn test_scan_typescript_client_call() {
        let source = b"metacall('sum', 1, 2)";
        let tree = parse(LangId::TypeScript, source);
        let sites = scan_file(LangId::TypeScript, &tree, source, Path::new("test.ts"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_tsx_client_call() {
        let source = b"metacall('sum', 1, 2)";
        let tree = parse(LangId::Tsx, source);
        let sites = scan_file(LangId::Tsx, &tree, source, Path::new("test.tsx"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_cpp_client_call() {
        let source = b"metacall(\"sum\", 1, 2);";
        let tree = parse(LangId::Cpp, source);
        let sites = scan_file(LangId::Cpp, &tree, source, Path::new("test.cpp"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_node_metacallfms() {
        let source = b"metacallfms('sum', '{\"a\":1}')";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_file(LangId::JavaScript, &tree, source, Path::new("test.js"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_rust_metacall_no_arg() {
        let source = b"metacall::metacall_no_arg(\"greet\")";
        let tree = parse(LangId::Rust, source);
        let sites = scan_file(LangId::Rust, &tree, source, Path::new("lib.rs"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("greet"));
    }

    #[test]
    fn test_scan_python_fstring_is_computed_name() {
        // An f-string is not a plain literal: the runtime name is computed,
        // so confidence must drop to 0.4.
        let source = b"metacall(f'fn_{suffix}', 1)";
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert!(sites[0].function_name.as_deref().unwrap().contains("fn_"));
        assert_eq!(sites[0].confidence, 0.4);
    }

    #[test]
    fn test_scan_javascript_template_string_is_computed_name() {
        let source = b"metacall(`fn_${suffix}`, 1)";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_file(LangId::JavaScript, &tree, source, Path::new("test.js"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].confidence, 0.4);
    }

    #[test]
    fn test_scan_metacall_handle_not_matched() {
        // metacall_handle(tag, name) has a per-port argument layout (tag
        // first in C/Node, handle first in Rust), so it is not matched.
        let py_source = b"metacall_handle('node', 'sum')";
        let py_tree = parse(LangId::Python, py_source);
        let py_sites = scan_file(LangId::Python, &py_tree, py_source, Path::new("test.py"));
        assert!(py_sites.is_empty());

        let c_source = b"metacall_handle(\"node\", \"sum\");";
        let c_tree = parse(LangId::C, c_source);
        let c_sites = scan_file(LangId::C, &c_tree, c_source, Path::new("test.c"));
        assert!(c_sites.is_empty());
    }

    #[test]
    fn test_scan_rust_client_call() {
        let source = b"metacall::metacall(\"sum\", &[1, 2])";
        let tree = parse(LangId::Rust, source);
        let sites = scan_file(LangId::Rust, &tree, source, Path::new("lib.rs"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_rust_from_single_file() {
        let source = b"metacall::load::from_single_file(\"py\", \"x.py\")";
        let tree = parse(LangId::Rust, source);
        let sites = scan_file(LangId::Rust, &tree, source, Path::new("lib.rs"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].scripts, vec!["x.py"]);
    }

    #[test]
    fn test_scan_ignores_metacall_inspect() {
        let source = b"metacall_inspect()";
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        assert!(sites.is_empty());
    }
}
