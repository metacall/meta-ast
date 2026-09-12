//! MetaCall call-site scanner.
//!
//! Emits one `CallSite` per MetaCall load or client call detected by tree-sitter queries.

use crate::graph::edge::CONFIDENCE_COMPUTED;
use crate::language::LangId;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use tree_sitter::{Node, Query, QueryCursor, StreamingIterator, Tree};

use serde::{Deserialize, Serialize};

/// Variant of a MetaCall call site: a load API or a client invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum CallSiteVariant {
    LoadFromFile,
    LoadFromMemory,
    LoadFromPackage,
    LoadFromConfiguration,
    ClientCall, // metacall, metacall_await, metacallfms, metacallv, metacallt,
                // metacall_function, metacall::metacall, Go Call/Await
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    pub confidence: f32,
}

impl CallSite {
    /// A load call site: the loaded scripts and, when the source spells one,
    /// the loader tag. A load never carries an invocation target.
    pub fn load(
        source_file: PathBuf,
        caller_lang: LangId,
        variant: CallSiteVariant,
        target_lang: Option<String>,
        scripts: Vec<String>,
        source_range: Option<crate::model::SourceRange>,
        confidence: f32,
    ) -> Self {
        Self {
            source_file,
            caller_lang,
            variant,
            target_lang,
            scripts,
            function_name: None,
            is_async: false,
            source_range,
            confidence,
        }
    }

    /// A client invocation: the target name is mandatory and no scripts apply.
    pub fn call(
        source_file: PathBuf,
        caller_lang: LangId,
        function_name: String,
        is_async: bool,
        source_range: Option<crate::model::SourceRange>,
        confidence: f32,
    ) -> Self {
        Self {
            source_file,
            caller_lang,
            variant: CallSiteVariant::ClientCall,
            target_lang: None,
            scripts: Vec::new(),
            function_name: Some(function_name),
            is_async,
            source_range,
            confidence,
        }
    }

    /// A client invocation whose target name is not visible in the source.
    ///
    /// The site is kept: it still proves the file calls into MetaCall, and
    /// dropping it would hide the file from the deployment plan.
    pub fn call_without_target(
        source_file: PathBuf,
        caller_lang: LangId,
        is_async: bool,
        source_range: Option<crate::model::SourceRange>,
        confidence: f32,
    ) -> Self {
        Self {
            source_file,
            caller_lang,
            variant: CallSiteVariant::ClientCall,
            target_lang: None,
            scripts: Vec::new(),
            function_name: None,
            is_async,
            source_range,
            confidence,
        }
    }
}

/// Exact client call names across the ports.
const CLIENT_NAMES: [&str; 20] = [
    "metacall",
    "metacall_await",
    "metacall_await_s",
    "metacall_no_arg",
    "metacall_untyped",
    "metacall_untyped_no_arg",
    "metacallfms",
    "metacallfms_await",
    "metacallv",
    "metacallv_s",
    "metacallt",
    "metacallt_s",
    "metacall_function",
    "Call",
    "CallUnsafe",
    "Await",
    "AwaitUnsafe",
    "LoadFromFile",
    "LoadFromMemory",
    "LoadFromPackage",
];

/// The one alternation every query predicate and `from_str` share.
static FUNCTION_NAME_PATTERN: LazyLock<String> = LazyLock::new(|| {
    let mut alternatives: Vec<String> = CLIENT_NAMES
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    alternatives.push("metacall_load_from_.*".to_string());
    alternatives.push("load_from_.*".to_string());
    alternatives.push("from_(file|single_file|memory|package|configuration)".to_string());
    alternatives.push("LoadFrom(File|Memory|Package|Configuration)".to_string());
    format!("^({})$", alternatives.join("|"))
});

/// Fill the shared function name predicate into a query template.
fn deploy_source(template: &str) -> String {
    template.replace("@FN@", &FUNCTION_NAME_PATTERN)
}

fn load_variant(suffix: &str) -> Option<CallSiteVariant> {
    match suffix {
        "file" | "single_file" => Some(CallSiteVariant::LoadFromFile),
        "memory" => Some(CallSiteVariant::LoadFromMemory),
        "package" => Some(CallSiteVariant::LoadFromPackage),
        "configuration" => Some(CallSiteVariant::LoadFromConfiguration),
        _ => None,
    }
}

impl CallSiteVariant {
    fn from_str(s: &str) -> Option<Self> {
        if let Some(suffix) = s.strip_prefix("metacall_load_from_") {
            return load_variant(suffix);
        }
        if let Some(suffix) = s.strip_prefix("from_") {
            return load_variant(suffix);
        }
        if let Some(suffix) = s.strip_prefix("load_from_") {
            return load_variant(suffix);
        }
        if let Some(suffix) = s.strip_prefix("LoadFrom") {
            return load_variant(&suffix.to_lowercase());
        }
        // metacall_handle excluded: its argument layout differs per port
        // (tag first in C/Node, handle first in Rust).
        if CLIENT_NAMES.contains(&s) {
            return Some(Self::ClientCall);
        }
        None
    }
}

fn is_async_call(name: &str) -> bool {
    matches!(
        name,
        "metacall_await" | "metacall_await_s" | "metacallfms_await" | "Await" | "AwaitUnsafe"
    )
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
        &deploy_source(
            r#"
(call
  function: (identifier) @fn_name
  arguments: (argument_list) @args
  (#match? @fn_name "@FN@"))
"#,
        ),
        "Python deploy",
    )
});

static JS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_javascript::LANGUAGE.into(),
        &deploy_source(
            r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args
  (#match? @fn_name "@FN@"))
"#,
        ),
        "JS deploy",
    )
});

static TS_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        &deploy_source(
            r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args
  (#match? @fn_name "@FN@"))
"#,
        ),
        "TS deploy",
    )
});

static TSX_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        &deploy_source(
            r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (arguments) @args
  (#match? @fn_name "@FN@"))
"#,
        ),
        "TSX deploy",
    )
});

static C_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_c::LANGUAGE.into(),
        &deploy_source(
            r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (argument_list) @args
  (#match? @fn_name "@FN@"))
"#,
        ),
        "C deploy",
    )
});

static CPP_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_cpp::LANGUAGE.into(),
        &deploy_source(
            r#"
(call_expression
  function: (identifier) @fn_name
  arguments: (argument_list) @args
  (#match? @fn_name "@FN@"))
"#,
        ),
        "CPP deploy",
    )
});

static RUST_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_rust::LANGUAGE.into(),
        &deploy_source(
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
  arguments: (arguments) @args
  (#match? @fn_name "@FN@"))
"#,
        ),
        "Rust deploy",
    )
});

static GO_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_go::LANGUAGE.into(),
        &deploy_source(
            r#"
(call_expression
  function: (selector_expression
    operand: (identifier) @pkg_name
    field: (field_identifier) @fn_name)
  arguments: (argument_list) @args
  (#match? @pkg_name "^metacall$")
  (#match? @fn_name "@FN@"))

; A one argument call parses as a type conversion in this grammar.
(type_conversion_expression
  (qualified_type
    (package_identifier) @pkg_name
    (type_identifier) @fn_name)
  operand: (_) @args
  (#match? @pkg_name "^metacall$")
  (#match? @fn_name "@FN@"))
"#,
        ),
        "Go deploy",
    )
});

static RUBY_QUERY: LazyLock<Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_ruby::LANGUAGE.into(),
        &deploy_source(
            r#"
(call
  method: (identifier) @fn_name
  arguments: (argument_list) @args
  (#match? @fn_name "@FN@"))
"#,
        ),
        "Ruby deploy",
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
        LangId::Ruby => &*RUBY_QUERY,
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

        for capture in mat.captures() {
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
                // keep the source text at computed confidence.
                if let Some(fn_node) = named_children.first() {
                    let text = get_node_text(*fn_node, source);
                    if is_plain_string(*fn_node) {
                        function_name = Some(strip_quotes(text));
                    } else {
                        function_name = Some(text.to_string());
                        confidence = CONFIDENCE_COMPUTED;
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
                        confidence = CONFIDENCE_COMPUTED;
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
                            confidence = CONFIDENCE_COMPUTED;
                        }
                    }
                }
            }

            let site = if variant == CallSiteVariant::ClientCall {
                match function_name {
                    Some(function_name) => CallSite::call(
                        path.to_path_buf(),
                        id,
                        function_name,
                        is_async,
                        source_range,
                        confidence,
                    ),
                    None => CallSite::call_without_target(
                        path.to_path_buf(),
                        id,
                        is_async,
                        source_range,
                        confidence,
                    ),
                }
            } else {
                CallSite::load(
                    path.to_path_buf(),
                    id,
                    variant,
                    target_lang,
                    scripts,
                    source_range,
                    confidence,
                )
            };
            call_sites.push(site);
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
    fn test_scan_rust_metacall_untyped_no_arg() {
        let source = b"metacall::metacall_untyped_no_arg(\"greet\")";
        let tree = parse(LangId::Rust, source);
        let sites = scan_file(LangId::Rust, &tree, source, Path::new("lib.rs"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("greet"));
        assert!(!sites[0].is_async);
    }

    #[test]
    fn test_scan_go_call_unsafe() {
        let source = b"metacall.CallUnsafe(\"sum\", 1, 2)";
        let tree = parse(LangId::Go, source);
        let sites = scan_file(LangId::Go, &tree, source, Path::new("main.go"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
        assert!(!sites[0].is_async);
    }

    #[test]
    fn test_scan_go_await_unsafe_is_async() {
        let source = b"metacall.AwaitUnsafe(\"sum\", resolve, reject, ctx)";
        let tree = parse(LangId::Go, source);
        let sites = scan_file(LangId::Go, &tree, source, Path::new("main.go"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert!(sites[0].is_async);
    }

    #[test]
    fn test_scan_c_metacallv_s() {
        let source = b"metacallv_s(\"sum\", args, size);";
        let tree = parse(LangId::C, source);
        let sites = scan_file(LangId::C, &tree, source, Path::new("test.c"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_c_metacall_await_s_is_async() {
        let source = b"metacall_await_s(\"sum\", args, size, resolve, reject, data);";
        let tree = parse(LangId::C, source);
        let sites = scan_file(LangId::C, &tree, source, Path::new("test.c"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert!(sites[0].is_async);
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

    /// Rust classification must not accept a look-alike helper name, and the
    /// genuine `metacall::load::*` names must still be detected.
    #[test]
    fn test_scan_rust_rejects_lookalike_helpers() {
        let source = br#"
fn f() { let a = reader.read_from_file("a"); }
fn g() { let b = loader.load_from_memory_cache("b"); }
fn h() { let c = copy_from_file_buffer("c"); }
fn i() { let d = cache_from_configuration("d"); }
"#;
        let tree = parse(LangId::Rust, source);
        let sites = scan_file(LangId::Rust, &tree, source, Path::new("lib.rs"));
        assert!(
            sites.is_empty(),
            "look-alike helpers must not be MetaCall sites: {:?}",
            sites
                .iter()
                .map(|s| (&s.variant, &s.scripts))
                .collect::<Vec<_>>()
        );

        let genuine = b"metacall::load::from_file(Tag::NodeJS, [\"index.js\"], None)";
        let tree = parse(LangId::Rust, genuine);
        let sites = scan_file(LangId::Rust, &tree, genuine, Path::new("lib.rs"));
        assert_eq!(sites.len(), 1, "the real load API must still be detected");
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
    }

    /// The script-language predicates must accept the full MetaCall client API,
    /// not only `metacall` and `metacall_await`.
    #[test]
    fn test_scan_python_accepts_the_full_client_api() {
        let source = br#"
metacallv("multiply", 2, 3)
metacallt("multiply", "int", "int")
metacall_no_arg()
metacall_untyped("multiply", 2, 3)
metacallfms_await("x")
metacall_await_s("x", 1)
"#;
        let tree = parse(LangId::Python, source);
        let sites = scan_file(LangId::Python, &tree, source, Path::new("test.py"));
        let names: Vec<&str> = sites
            .iter()
            .filter_map(|s| s.function_name.as_deref().or(s.target_lang.as_deref()))
            .collect();
        assert_eq!(
            sites.len(),
            6,
            "every client API name must be scanned, got {names:?}"
        );
    }

    /// The Go package selector must match the package exactly.
    ///
    /// Two arguments are required: with one argument tree-sitter-go parses
    /// `pkg.Fn(x)` as a type conversion, not a call.
    #[test]
    fn test_scan_go_single_argument_call_is_detected() {
        // This grammar parses a one argument call as a type conversion.
        let source = b"package main\nfunc main() { metacall.Call(handler) }";
        let tree = parse(LangId::Go, source);
        let sites = scan_file(LangId::Go, &tree, source, Path::new("main.go"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("handler"));
    }

    #[test]
    fn test_scan_go_rejects_a_lookalike_package() {
        let source = b"metacallmock.Call(\"x\", 1)";
        let tree = parse(LangId::Go, source);
        let sites = scan_file(LangId::Go, &tree, source, Path::new("main.go"));
        assert!(sites.is_empty(), "metacallmock is not the MetaCall package");

        let genuine = b"metacall.Call(\"x\", 1)";
        let tree = parse(LangId::Go, genuine);
        let sites = scan_file(LangId::Go, &tree, genuine, Path::new("main.go"));
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
    }
}
