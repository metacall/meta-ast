//! MetaCall call-site scanner.
//!
//! Emits one `CallSite` per MetaCall load or client call detected by tree-sitter queries.

use crate::deploy::bindings::{Decl, FileBindings};
use crate::graph::edge::CONFIDENCE_COMPUTED;
use crate::language::LangId;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use crate::language::common::query_from;
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

/// True when a member call's receiver names the MetaCall module: the bare name,
/// a file alias, or the inline `require("metacall")`, through any wrapping
/// parentheses or TypeScript non-null assertion.
fn receiver_is_module(source: &[u8], node: Node, bindings: &FileBindings) -> bool {
    let node = crate::deploy::bindings::unwrap_value(node);
    if crate::deploy::bindings::is_require_metacall(node, source) {
        return true;
    }
    if !crate::deploy::bindings::is_name_node(node.kind()) {
        return false;
    }
    let name = crate::deploy::bindings::text(node, source);
    match bindings.resolve(node, name) {
        Some(Decl::Module) => true,
        Some(_) => false,
        // No declaration anywhere: the bare module name still counts.
        None => name == "metacall",
    }
}

/// The runtime entry point a bare call names, when any.
///
/// A renamed entry point resolves through its import; otherwise the callee
/// text must name one directly.
fn bare_target(callee: Node, name: &str, bindings: &FileBindings) -> Option<String> {
    let callee = crate::deploy::bindings::unwrap_value(callee);
    if crate::deploy::bindings::is_name_node(callee.kind()) {
        match bindings.resolve(callee, name) {
            Some(Decl::Member(member)) => {
                return CallSiteVariant::from_str(&member).map(|_| member);
            }
            Some(_) => return None,
            None => {}
        }
    }
    CallSiteVariant::from_str(name).map(|_| name.to_string())
}

fn collect_strings_recursive(node: Node, source: &[u8], scripts: &mut Vec<String>) {
    let kind = node.kind();
    if kind.contains("string") || kind == "string_literal" {
        scripts.push(
            crate::deploy::bindings::strip_quote_pair(crate::deploy::bindings::text(node, source))
                .to_string(),
        );
    } else {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.is_named() {
                collect_strings_recursive(child, source, scripts);
            }
        }
    }
}

static PYTHON_QUERY: LazyLock<Result<Query, String>> = LazyLock::new(|| {
    crate::language::common::compile_query_checked(
        &tree_sitter_python::LANGUAGE.into(),
        r#"
(call
  function: [(identifier) (parenthesized_expression)] @fn_name
  arguments: (argument_list) @args)
(call
  function: (attribute
    object: [(identifier) (parenthesized_expression)] @recv
    attribute: (identifier) @fn_name)
  arguments: (argument_list) @args)
"#,
        "Python deploy",
    )
});

static JS_QUERY: LazyLock<Result<Query, String>> = LazyLock::new(|| {
    crate::language::common::compile_query_checked(
        &tree_sitter_javascript::LANGUAGE.into(),
        r#"
(call_expression
  function: [(identifier) (parenthesized_expression)] @fn_name
  arguments: (arguments) @args)
(call_expression
  function: (member_expression
    object: [(identifier) (call_expression) (parenthesized_expression)] @recv
    property: (property_identifier) @fn_name)
  arguments: (arguments) @args)
"#,
        "JS deploy",
    )
});

static TS_QUERY: LazyLock<Result<Query, String>> = LazyLock::new(|| {
    crate::language::common::compile_query_checked(
        &tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        r#"
(call_expression
  function: [(identifier) (parenthesized_expression) (non_null_expression)
               (as_expression) (satisfies_expression) (type_assertion)] @fn_name
  arguments: (arguments) @args)
(call_expression
  function: (member_expression
    object: [(identifier) (call_expression) (parenthesized_expression) (non_null_expression)
             (as_expression) (satisfies_expression) (type_assertion)] @recv
    property: (property_identifier) @fn_name)
  arguments: (arguments) @args)
"#,
        "TS deploy",
    )
});

static TSX_QUERY: LazyLock<Result<Query, String>> = LazyLock::new(|| {
    crate::language::common::compile_query_checked(
        &tree_sitter_typescript::LANGUAGE_TSX.into(),
        r#"
(call_expression
  function: [(identifier) (parenthesized_expression) (non_null_expression)
               (as_expression) (satisfies_expression)] @fn_name
  arguments: (arguments) @args)
(call_expression
  function: (member_expression
    object: [(identifier) (call_expression) (parenthesized_expression) (non_null_expression)
             (as_expression) (satisfies_expression)] @recv
    property: (property_identifier) @fn_name)
  arguments: (arguments) @args)
"#,
        "TSX deploy",
    )
});

static C_QUERY: LazyLock<Result<Query, String>> = LazyLock::new(|| {
    crate::language::common::compile_query_checked(
        &tree_sitter_c::LANGUAGE.into(),
        r#"
(call_expression
  function: [(identifier) (parenthesized_expression)] @fn_name
  arguments: (argument_list) @args)
"#,
        "C deploy",
    )
});

static CPP_QUERY: LazyLock<Result<Query, String>> = LazyLock::new(|| {
    crate::language::common::compile_query_checked(
        &tree_sitter_cpp::LANGUAGE.into(),
        r#"
(call_expression
  function: [(identifier) (parenthesized_expression)] @fn_name
  arguments: (argument_list) @args)
(call_expression
  function: (qualified_identifier
    scope: (namespace_identifier) @recv
    name: (identifier) @fn_name)
  arguments: (argument_list) @args)
"#,
        "CPP deploy",
    )
});

static RUST_QUERY: LazyLock<Result<Query, String>> = LazyLock::new(|| {
    crate::language::common::compile_query_checked(
        &tree_sitter_rust::LANGUAGE.into(),
        r#"
(call_expression
  function: [
    (scoped_identifier
        path: (identifier) @recv
        name: (identifier) @fn_name)
    (scoped_identifier
        path: (scoped_identifier path: (identifier) @recv name: (identifier))
        name: (identifier) @fn_name)
  ]
  arguments: (arguments) @args)
(call_expression
  function: [(identifier) (parenthesized_expression)] @fn_name
  arguments: (arguments) @args)
"#,
        "Rust deploy",
    )
});

static GO_QUERY: LazyLock<Result<Query, String>> = LazyLock::new(|| {
    crate::language::common::compile_query_checked(
        &tree_sitter_go::LANGUAGE.into(),
        r#"
(call_expression
  function: (selector_expression
    operand: [(identifier) (parenthesized_expression)] @recv
    field: (field_identifier) @fn_name)
  arguments: (argument_list) @args)

; A one argument call parses as a type conversion in this grammar.
(type_conversion_expression
  (qualified_type
    (package_identifier) @recv
    (type_identifier) @fn_name)
  operand: (_) @args)
"#,
        "Go deploy",
    )
});

static RUBY_QUERY: LazyLock<Result<Query, String>> = LazyLock::new(|| {
    crate::language::common::compile_query_checked(
        &tree_sitter_ruby::LANGUAGE.into(),
        r#"
(call
  !receiver
  method: (identifier) @fn_name
  arguments: (argument_list) @args)
(call
  receiver: [(identifier) (parenthesized_statements)] @recv
  method: (identifier) @fn_name
  arguments: (argument_list) @args)
"#,
        "Ruby deploy",
    )
});

pub fn scan_file(
    id: LangId,
    tree: &Tree,
    source: &[u8],
    path: &Path,
) -> Result<Vec<CallSite>, crate::Error> {
    let query = match id {
        LangId::Python => query_from(&PYTHON_QUERY, id)?,
        LangId::JavaScript => query_from(&JS_QUERY, id)?,
        LangId::TypeScript => query_from(&TS_QUERY, id)?,
        LangId::Tsx => query_from(&TSX_QUERY, id)?,
        LangId::C => query_from(&C_QUERY, id)?,
        LangId::Cpp => query_from(&CPP_QUERY, id)?,
        LangId::Rust => query_from(&RUST_QUERY, id)?,
        LangId::Go => query_from(&GO_QUERY, id)?,
        LangId::Ruby => query_from(&RUBY_QUERY, id)?,
    };

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, tree.root_node(), source);

    let mut call_sites = Vec::new();

    // Capture indices are static query-shape facts; a missing name means a
    // malformed query constant, not runtime data. Bail out rather than panic.
    let Some(fn_name_idx) = query.capture_index_for_name("fn_name") else {
        return Ok(call_sites);
    };
    let Some(args_idx) = query.capture_index_for_name("args") else {
        return Ok(call_sites);
    };
    let recv_idx = query.capture_index_for_name("recv");
    let bindings = FileBindings::collect(id, tree, source);

    while let Some(mat) = matches.next() {
        let mut target_lang = None;
        let mut scripts = Vec::new();
        let mut confidence = 1.0;
        let mut name = "";

        let mut args_node = None;
        let mut receiver = None;
        let mut callee = None;

        for capture in mat.captures() {
            if capture.index == fn_name_idx {
                callee = Some(capture.node);
                name = crate::deploy::bindings::text(
                    crate::deploy::bindings::unwrap_value(capture.node),
                    source,
                );
            } else if capture.index == args_idx {
                args_node = Some(capture.node);
            } else if recv_idx == Some(capture.index) {
                receiver = Some(capture.node);
            }
        }

        // A member call counts only on the MetaCall module; a bare call counts
        // when it names an entry point, directly or through a rename.
        let resolved = match receiver {
            Some(node) => receiver_is_module(source, node, &bindings).then(|| name.to_string()),
            None => callee.and_then(|node| bare_target(node, name, &bindings)),
        };
        let Some(resolved) = resolved else {
            continue;
        };
        let resolved = resolved.as_str();
        let Some(variant) = CallSiteVariant::from_str(resolved) else {
            continue;
        };

        if let Some(args) = args_node {
            let is_async = is_async_call(resolved);
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
                    if let Some(literal) = crate::deploy::bindings::string_text(*fn_node, source) {
                        function_name = Some(literal.to_string());
                    } else {
                        function_name =
                            Some(crate::deploy::bindings::text(*fn_node, source).to_string());
                        confidence = CONFIDENCE_COMPUTED;
                    }
                }
            } else if variant == CallSiteVariant::LoadFromConfiguration {
                // A configuration load takes one argument: the config path.
                // It travels in scripts, where both config readers look;
                // the tag stays empty because the language comes from the
                // configuration itself.
                if let Some(path_node) = named_children.first()
                    && let Some(path_text) =
                        crate::deploy::bindings::string_text(*path_node, source)
                {
                    scripts.push(path_text.to_string());
                }
            } else {
                if let Some(lang_node) = named_children.first() {
                    if let Some(tag) = crate::deploy::bindings::string_text(*lang_node, source) {
                        target_lang = Some(tag.to_string());
                    } else {
                        // A Rust loader path spells `Tag::NodeJS`: the tag is
                        // the last segment. A known tag stays certain.
                        let raw = crate::deploy::bindings::text(*lang_node, source);
                        let segment = raw.rsplit("::").next().unwrap_or(raw);
                        target_lang = Some(segment.to_string());
                        if crate::deploy::tags::from_metacall_tag(segment).is_none() {
                            confidence = CONFIDENCE_COMPUTED;
                        }
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
                    } else if let Some(script) =
                        crate::deploy::bindings::string_text(*scripts_node, source)
                    {
                        scripts.push(script.to_string());
                    }
                    // Anything else names a variable holding the scripts, such
                    // as the C array pointer in (tag, paths, size, handle).
                    // Its text is not a path, so it contributes no scripts
                    // instead of a phantom node named after the variable.
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

    Ok(call_sites)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A built-in query must compile; a failure is a defect, not test data.
    fn scan_sites(id: LangId, tree: &Tree, source: &[u8], path: &str) -> Vec<CallSite> {
        let result = scan_file(id, tree, source, Path::new(path));
        assert!(
            result.is_ok(),
            "the built-in query compiles: {:?}",
            result.as_ref().err()
        );
        result.unwrap()
    }
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
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
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
        let sites = scan_sites(LangId::JavaScript, &tree, source, "test.js");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("py"));
        assert_eq!(sites[0].scripts, vec!["sum.py"]);
    }

    #[test]
    fn test_scan_rust() {
        let source = b"metacall::load_from_file(\"py\", [\"sum.py\"])";
        let tree = parse(LangId::Rust, source);
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("py"));
        assert_eq!(sites[0].scripts, vec!["sum.py"]);
    }

    #[test]
    fn test_scan_computed_args() {
        let source = b"metacall_load_from_file(LANG, ['sum.js'])";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].confidence, 0.4);
        assert_eq!(sites[0].target_lang.as_deref(), Some("LANG"));
    }

    #[test]
    fn test_scan_rust_bare_name() {
        // After `use metacall::metacall_load_from_file`, the call is bare.
        let source = b"metacall_load_from_file(\"py\", [\"sum.py\"])";
        let tree = parse(LangId::Rust, source);
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("py"));
        assert_eq!(sites[0].scripts, vec!["sum.py"]);
    }

    #[test]
    fn test_scan_python_load_from_memory() {
        let source = b"metacall_load_from_memory('node', 'console.log(\"hi\")')";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromMemory);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
    }

    #[test]
    fn test_scan_python_load_from_package() {
        let source = b"metacall_load_from_package('node', 'express')";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromPackage);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
        assert_eq!(sites[0].scripts, vec!["express"]);
    }

    #[test]
    fn test_scan_load_from_configuration_carries_the_path_in_scripts() {
        let source = b"metacall_load_from_configuration('cfg/deploy.json')";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromConfiguration);
        assert_eq!(sites[0].scripts, vec!["cfg/deploy.json"]);
        assert_eq!(sites[0].target_lang, None);
    }

    #[test]
    fn test_scan_go_load_from_memory() {
        let source = b"metacall.LoadFromMemory(\"node\", []string{\"const x = 1;\"})";
        let tree = parse(LangId::Go, source);
        let sites = scan_sites(LangId::Go, &tree, source, "main.go");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromMemory);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
    }

    #[test]
    fn test_scan_python_client_call() {
        let source = b"metacall('sum', 1, 2)";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
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
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert!(sites[0].is_async);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_python_computed_function_name() {
        let source = b"metacall(fn_name, 1)";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("fn_name"));
        assert_eq!(sites[0].confidence, 0.4);
    }

    #[test]
    fn test_scan_javascript_client_call() {
        let source = b"metacall('sum', 1, 2)";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_sites(LangId::JavaScript, &tree, source, "test.js");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_c_load_from_file() {
        let source = b"metacall_load_from_file(\"node\", paths, size, &handle);";
        let tree = parse(LangId::C, source);
        let sites = scan_sites(LangId::C, &tree, source, "test.c");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
    }

    #[test]
    fn test_scan_c_client_call() {
        let source = b"metacall(\"sum\", 1, 2);\nmetacallv(\"sum\", args);";
        let tree = parse(LangId::C, source);
        let sites = scan_sites(LangId::C, &tree, source, "test.c");
        assert_eq!(sites.len(), 2);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[1].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_go_client_call() {
        let source = b"metacall.Call(\"sum\", 1, 2)";
        let tree = parse(LangId::Go, source);
        let sites = scan_sites(LangId::Go, &tree, source, "main.go");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_go_client_await() {
        let source = b"metacall.Await(\"sum\", resolve, reject, ctx, 1)";
        let tree = parse(LangId::Go, source);
        let sites = scan_sites(LangId::Go, &tree, source, "main.go");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert!(sites[0].is_async);
    }

    #[test]
    fn test_scan_typescript_client_call() {
        let source = b"metacall('sum', 1, 2)";
        let tree = parse(LangId::TypeScript, source);
        let sites = scan_sites(LangId::TypeScript, &tree, source, "test.ts");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_tsx_client_call() {
        let source = b"metacall('sum', 1, 2)";
        let tree = parse(LangId::Tsx, source);
        let sites = scan_sites(LangId::Tsx, &tree, source, "test.tsx");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_cpp_client_call() {
        let source = b"metacall(\"sum\", 1, 2);";
        let tree = parse(LangId::Cpp, source);
        let sites = scan_sites(LangId::Cpp, &tree, source, "test.cpp");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_node_metacallfms() {
        let source = b"metacallfms('sum', '{\"a\":1}')";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_sites(LangId::JavaScript, &tree, source, "test.js");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_rust_metacall_no_arg() {
        let source = b"metacall::metacall_no_arg(\"greet\")";
        let tree = parse(LangId::Rust, source);
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("greet"));
    }

    #[test]
    fn test_scan_rust_metacall_untyped_no_arg() {
        let source = b"metacall::metacall_untyped_no_arg(\"greet\")";
        let tree = parse(LangId::Rust, source);
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("greet"));
        assert!(!sites[0].is_async);
    }

    #[test]
    fn test_scan_go_call_unsafe() {
        let source = b"metacall.CallUnsafe(\"sum\", 1, 2)";
        let tree = parse(LangId::Go, source);
        let sites = scan_sites(LangId::Go, &tree, source, "main.go");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
        assert!(!sites[0].is_async);
    }

    #[test]
    fn test_scan_go_await_unsafe_is_async() {
        let source = b"metacall.AwaitUnsafe(\"sum\", resolve, reject, ctx)";
        let tree = parse(LangId::Go, source);
        let sites = scan_sites(LangId::Go, &tree, source, "main.go");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert!(sites[0].is_async);
    }

    #[test]
    fn test_scan_c_metacallv_s() {
        let source = b"metacallv_s(\"sum\", args, size);";
        let tree = parse(LangId::C, source);
        let sites = scan_sites(LangId::C, &tree, source, "test.c");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_c_metacall_await_s_is_async() {
        let source = b"metacall_await_s(\"sum\", args, size, resolve, reject, data);";
        let tree = parse(LangId::C, source);
        let sites = scan_sites(LangId::C, &tree, source, "test.c");
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
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert!(sites[0].function_name.as_deref().unwrap().contains("fn_"));
        assert_eq!(sites[0].confidence, 0.4);
    }

    #[test]
    fn test_scan_javascript_template_string_is_computed_name() {
        let source = b"metacall(`fn_${suffix}`, 1)";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_sites(LangId::JavaScript, &tree, source, "test.js");
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
        let py_sites = scan_sites(LangId::Python, &py_tree, py_source, "test.py");
        assert!(py_sites.is_empty());

        let c_source = b"metacall_handle(\"node\", \"sum\");";
        let c_tree = parse(LangId::C, c_source);
        let c_sites = scan_sites(LangId::C, &c_tree, c_source, "test.c");
        assert!(c_sites.is_empty());
    }

    #[test]
    fn test_scan_rust_client_call() {
        let source = b"metacall::metacall(\"sum\", &[1, 2])";
        let tree = parse(LangId::Rust, source);
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));
    }

    #[test]
    fn test_scan_rust_from_single_file() {
        let source = b"metacall::load::from_single_file(\"py\", \"x.py\")";
        let tree = parse(LangId::Rust, source);
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].scripts, vec!["x.py"]);
    }

    #[test]
    fn test_scan_ignores_metacall_inspect() {
        let source = b"metacall_inspect()";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
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
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
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
        let sites = scan_sites(LangId::Rust, &tree, genuine, "lib.rs");
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
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
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
        let sites = scan_sites(LangId::Go, &tree, source, "main.go");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some("handler"));
    }

    #[test]
    fn test_scan_go_rejects_a_lookalike_package() {
        let source = b"metacallmock.Call(\"x\", 1)";
        let tree = parse(LangId::Go, source);
        let sites = scan_sites(LangId::Go, &tree, source, "main.go");
        assert!(sites.is_empty(), "metacallmock is not the MetaCall package");

        let genuine = b"metacall.Call(\"x\", 1)";
        let tree = parse(LangId::Go, genuine);
        let sites = scan_sites(LangId::Go, &tree, genuine, "main.go");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
    }

    #[test]
    fn test_scan_c_array_pointer_contributes_no_scripts() {
        let source = b"metacall_load_from_file(\"node\", paths, size, &handle);";
        let tree = parse(LangId::C, source);
        let sites = scan_sites(LangId::C, &tree, source, "test.c");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
        assert!(
            sites[0].scripts.is_empty(),
            "a variable is not a path: {:?}",
            sites[0].scripts
        );
        assert_eq!(sites[0].confidence, 1.0);
    }

    #[test]
    fn test_scan_python_variable_scripts_contribute_no_scripts() {
        let source = b"metacall_load_from_file('node', scripts)";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert!(
            sites[0].scripts.is_empty(),
            "a variable is not a path: {:?}",
            sites[0].scripts
        );
    }

    #[test]
    fn test_scan_rust_tag_path_resolves() {
        let source = b"metacall::load::from_file(Tag::NodeJS, [\"index.js\"], None)";
        let tree = parse(LangId::Rust, source);
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("NodeJS"));
        assert_eq!(sites[0].scripts, vec!["index.js"]);
        assert_eq!(sites[0].confidence, 1.0);
    }

    #[test]
    fn test_scan_rust_rejects_a_lookalike_path() {
        let source = b"other::load_from_file(\"py\", [\"sum.py\"])";
        let tree = parse(LangId::Rust, source);
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
        assert!(sites.is_empty(), "other:: is not the metacall path");
    }

    #[test]
    fn test_scan_python_member_load_is_detected() {
        let source = b"metacall.load_from_file('node', ['a.py'])";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
        assert_eq!(sites[0].scripts, vec!["a.py"]);
    }

    #[test]
    fn test_scan_python_member_call_on_another_object_is_ignored() {
        let source = b"db.Call('x')";
        let tree = parse(LangId::Python, source);
        let sites = scan_sites(LangId::Python, &tree, source, "test.py");
        assert!(sites.is_empty(), "db.Call is not a MetaCall invocation");
    }

    #[test]
    fn test_scan_javascript_member_load_is_detected() {
        let source = b"metacall.load_from_file('node', ['a.js'])";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_sites(LangId::JavaScript, &tree, source, "test.js");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].scripts, vec!["a.js"]);
    }

    #[test]
    fn test_scan_javascript_member_call_on_another_object_is_ignored() {
        let source = b"db.Call('x')";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_sites(LangId::JavaScript, &tree, source, "test.js");
        assert!(sites.is_empty(), "db.Call is not a MetaCall invocation");
    }

    #[test]
    fn test_scan_cpp_qualified_load_is_detected() {
        let source = b"metacall::load_from_file(\"node\", scripts);";
        let tree = parse(LangId::Cpp, source);
        let sites = scan_sites(LangId::Cpp, &tree, source, "test.cpp");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("node"));
    }

    /// A renamed module receiver is accepted; a rename of anything else is not.
    fn assert_one_client_call(id: LangId, source: &[u8], path: &str, expected: &str) {
        let tree = parse(id, source);
        let sites = scan_sites(id, &tree, source, path);
        assert_eq!(sites.len(), 1, "one call site expected: {sites:?}");
        assert_eq!(sites[0].variant, CallSiteVariant::ClientCall);
        assert_eq!(sites[0].function_name.as_deref(), Some(expected));
    }

    #[test]
    fn test_scan_javascript_require_alias_binds_the_module() {
        assert_one_client_call(
            LangId::JavaScript,
            b"const mc = require(\"metacall\");\nmc.metacall(\"sum\", 1, 2);",
            "test.js",
            "sum",
        );
    }

    #[test]
    fn test_scan_javascript_require_destructuring_renames_a_member() {
        assert_one_client_call(
            LangId::JavaScript,
            b"const { metacall: m } = require(\"metacall\");\nm(\"sum\", 1, 2);",
            "test.js",
            "sum",
        );
    }

    #[test]
    fn test_scan_javascript_require_alias_loads() {
        let source =
            b"const mc = require(\"metacall\");\nmc.metacall_load_from_file(\"py\", [\"x.py\"]);";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_sites(LangId::JavaScript, &tree, source, "test.js");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("py"));
        assert_eq!(sites[0].scripts, vec!["x.py"]);
    }

    #[test]
    fn test_scan_javascript_import_forms_bind_names() {
        assert_one_client_call(
            LangId::JavaScript,
            b"import * as mc from \"metacall\";\nmc.metacall(\"sum\", 1, 2);",
            "test.mjs",
            "sum",
        );
        assert_one_client_call(
            LangId::JavaScript,
            b"import mc from \"metacall\";\nmc.metacall(\"sum\", 1, 2);",
            "test.mjs",
            "sum",
        );
        assert_one_client_call(
            LangId::JavaScript,
            b"import { metacall as m } from \"metacall\";\nm(\"sum\", 1, 2);",
            "test.mjs",
            "sum",
        );
    }

    #[test]
    fn test_scan_javascript_inline_require_receiver() {
        assert_one_client_call(
            LangId::JavaScript,
            b"require(\"metacall\").metacall(\"sum\", 1, 2);",
            "test.js",
            "sum",
        );
    }

    /// Wrapping parentheses and the TypeScript non-null assertion keep the receiver.
    #[test]
    fn test_scan_typescript_wrapped_receivers() {
        assert_one_client_call(
            LangId::TypeScript,
            b"import * as mc from \"metacall\";\nmc!.metacall(\"sum\", 1, 2);",
            "test.ts",
            "sum",
        );
        assert_one_client_call(
            LangId::TypeScript,
            b"import * as mc from \"metacall\";\n(mc).metacall(\"sum\", 1, 2);",
            "test.ts",
            "sum",
        );
        assert_one_client_call(
            LangId::TypeScript,
            b"import * as mc from \"metacall\";\n((mc!)).metacall(\"sum\", 1, 2);",
            "test.ts",
            "sum",
        );
        assert_one_client_call(
            LangId::Tsx,
            b"import * as mc from \"metacall\";\nmc!.metacall(\"sum\", 1, 2);",
            "test.tsx",
            "sum",
        );
    }

    #[test]
    fn test_scan_wrapped_load_and_inline_require_receivers() {
        let source =
            b"import * as mc from \"metacall\";\n(mc).metacall_load_from_file(\"py\", [\"x.py\"]);";
        let tree = parse(LangId::TypeScript, source);
        let sites = scan_sites(LangId::TypeScript, &tree, source, "test.ts");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].scripts, vec!["x.py"]);

        assert_one_client_call(
            LangId::TypeScript,
            b"(require(\"metacall\")).metacall(\"sum\", 1, 2);",
            "test.ts",
            "sum",
        );
        assert_one_client_call(
            LangId::JavaScript,
            b"const mc = require(\"metacall\");\n(mc).metacall(\"sum\", 1, 2);",
            "test.js",
            "sum",
        );
    }

    /// Every supported language peels the wrappers its grammar can produce.
    #[test]
    fn test_scan_typescript_type_wrappers() {
        for source in [
            &b"import * as mc from \"metacall\";\n(mc as any).metacall(\"sum\", 1, 2);"[..],
            &b"import * as mc from \"metacall\";\n(mc satisfies Shape).metacall(\"sum\", 1, 2);"[..],
            &b"import * as mc from \"metacall\";\n(<any>mc).metacall(\"sum\", 1, 2);"[..],
            &b"import * as mc from \"metacall\";\n((mc! as any)).metacall(\"sum\", 1, 2);"[..],
        ] {
            assert_one_client_call(LangId::TypeScript, source, "test.ts", "sum");
        }
        assert_one_client_call(
            LangId::Tsx,
            b"import * as mc from \"metacall\";\n(mc as any).metacall(\"sum\", 1, 2);",
            "test.tsx",
            "sum",
        );
    }

    #[test]
    fn test_scan_python_parenthesized_receiver() {
        assert_one_client_call(
            LangId::Python,
            b"import metacall as mc\n(mc).metacall(\"sum\", 1, 2)",
            "test.py",
            "sum",
        );
        assert_one_client_call(
            LangId::Python,
            b"from metacall import metacall as m\n((m))(\"sum\", 1, 2)",
            "test.py",
            "sum",
        );
    }

    #[test]
    fn test_scan_go_parenthesized_receiver() {
        assert_one_client_call(
            LangId::Go,
            b"import mc \"metacall\"\n\nfunc main() { (mc).Call(\"sum\", 1, 2) }",
            "main.go",
            "sum",
        );
    }

    /// Ruby calls carry a receiver as a field; only the module may have one.
    #[test]
    fn test_scan_ruby_receivers() {
        let bare = b"metacall(\"sum\", 1, 2)\n";
        let tree = parse(LangId::Ruby, bare);
        let sites = scan_sites(LangId::Ruby, &tree, bare, "test.rb");
        assert_eq!(sites.len(), 1, "a bare call is a client call: {sites:?}");
        assert_eq!(sites[0].function_name.as_deref(), Some("sum"));

        assert_one_client_call(
            LangId::Ruby,
            b"metacall.metacall(\"sum\", 1, 2)\n",
            "test.rb",
            "sum",
        );
        assert_one_client_call(
            LangId::Ruby,
            b"(metacall).metacall(\"sum\", 1, 2)\n",
            "test.rb",
            "sum",
        );
    }

    #[test]
    fn test_scan_ruby_receiver_of_another_object_is_ignored() {
        let source = b"db.metacall('x')\nlogger.metacall_load_from_file('py', ['x.py'])\n";
        let tree = parse(LangId::Ruby, source);
        let sites = scan_sites(LangId::Ruby, &tree, source, "test.rb");
        assert!(
            sites.is_empty(),
            "a receiver must be the module itself: {sites:?}"
        );
    }

    /// Two grammars cannot wrap a module receiver at all, and the parse shows it.
    /// A wrapped callee is the same call: parentheses keep the value, and the
    /// TypeScript type wrappers do too.
    #[test]
    fn test_scan_wrapped_callees() {
        assert_one_client_call(
            LangId::JavaScript,
            b"(metacall)(\"sum\", 1, 2);",
            "test.js",
            "sum",
        );
        assert_one_client_call(
            LangId::TypeScript,
            b"(metacall as any)(\"sum\", 1, 2);",
            "test.ts",
            "sum",
        );
        assert_one_client_call(
            LangId::C,
            b"void f() { (metacall)(\"sum\", 1, 2); }",
            "test.c",
            "sum",
        );
        assert_one_client_call(
            LangId::Cpp,
            b"void f() { (metacall)(\"sum\", 1, 2); }",
            "test.cpp",
            "sum",
        );
        assert_one_client_call(
            LangId::Rust,
            b"fn main() { (metacall)(\"sum\", &[1, 2]); }",
            "lib.rs",
            "sum",
        );
    }

    #[test]
    fn test_scan_wrapped_callee_of_another_name_is_ignored() {
        let source = b"(db)(\"x\");\n(log)(1);\n";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_sites(LangId::JavaScript, &tree, source, "test.js");
        assert!(
            sites.is_empty(),
            "wrapping must not admit another callee: {sites:?}"
        );
    }

    #[test]
    fn test_scan_unwrappable_receivers_stay_undetected() {
        let rust = b"use metacall as mc;\nfn main() { (mc)::metacall(\"sum\", &[1, 2]); }";
        let tree = parse(LangId::Rust, rust);
        assert!(
            scan_sites(LangId::Rust, &tree, rust, "lib.rs").is_empty(),
            "a parenthesized Rust path is a syntax error, not a receiver"
        );

        let cpp = b"namespace mc = metacall;\nvoid f() { (mc)::load_from_file(\"py\", \"x.py\"); }";
        let tree = parse(LangId::Cpp, cpp);
        assert!(
            scan_sites(LangId::Cpp, &tree, cpp, "main.cpp").is_empty(),
            "a parenthesized C++ namespace is a cast, not a receiver"
        );
    }

    #[test]
    fn test_scan_wrapped_receivers_of_another_module_are_ignored() {
        let source = b"const db = require(\"db\");\n(db).Call('x');\n(require(\"metacall-client\")).metacall('x');\n(db!).Call('x');";
        let tree = parse(LangId::TypeScript, source);
        let sites = scan_sites(LangId::TypeScript, &tree, source, "test.ts");
        assert!(
            sites.is_empty(),
            "wrapping must not admit another module: {sites:?}"
        );
    }

    #[test]
    fn test_scan_javascript_alias_of_another_module_is_ignored() {
        let source = b"const db = require(\"db\");\ndb.Call('x');\nconst m = require(\"metacall-client\");\nm.metacall('x');";
        let tree = parse(LangId::JavaScript, source);
        let sites = scan_sites(LangId::JavaScript, &tree, source, "test.js");
        assert!(
            sites.is_empty(),
            "only the metacall module aliases count: {sites:?}"
        );
    }

    #[test]
    fn test_scan_typescript_alias_forms() {
        assert_one_client_call(
            LangId::TypeScript,
            b"import * as mc from \"metacall\";\nmc.metacall(\"sum\", 1, 2);",
            "test.ts",
            "sum",
        );
        assert_one_client_call(
            LangId::Tsx,
            b"const mc = require(\"metacall\");\nmc.metacall(\"sum\", 1, 2);",
            "test.tsx",
            "sum",
        );
    }

    #[test]
    fn test_scan_python_alias_forms() {
        assert_one_client_call(
            LangId::Python,
            b"import metacall as mc\nmc.metacall(\"sum\", 1, 2)",
            "test.py",
            "sum",
        );
        assert_one_client_call(
            LangId::Python,
            b"from metacall import metacall as m\nm(\"sum\", 1, 2)",
            "test.py",
            "sum",
        );
    }

    #[test]
    fn test_scan_go_import_alias() {
        assert_one_client_call(
            LangId::Go,
            b"import mc \"metacall\"\n\nfunc main() { mc.Call(\"sum\", 1, 2) }",
            "main.go",
            "sum",
        );
    }

    #[test]
    fn test_scan_rust_use_alias() {
        assert_one_client_call(
            LangId::Rust,
            b"use metacall as mc;\n\nfn main() { mc::metacall(\"sum\", &[1, 2]); }",
            "lib.rs",
            "sum",
        );
    }

    #[test]
    fn test_scan_rust_use_alias_of_a_member_keeps_its_variant() {
        let source =
            b"use metacall::load_from_file as load;\n\nfn main() { load(\"py\", \"x.py\"); }";
        let tree = parse(LangId::Rust, source);
        let sites = scan_sites(LangId::Rust, &tree, source, "lib.rs");
        assert_eq!(sites.len(), 1, "one load site expected: {sites:?}");
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].target_lang.as_deref(), Some("py"));
        assert_eq!(sites[0].scripts, vec!["x.py"]);
    }

    #[test]
    fn test_scan_cpp_namespace_alias() {
        let source = b"namespace mc = metacall;\nmc::load_from_file(\"py\", \"x.py\");";
        let tree = parse(LangId::Cpp, source);
        let sites = scan_sites(LangId::Cpp, &tree, source, "main.cpp");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].variant, CallSiteVariant::LoadFromFile);
        assert_eq!(sites[0].scripts, vec!["x.py"]);
    }

    #[test]
    fn strip_quotes_strips_one_pair() {
        let strip = crate::deploy::bindings::strip_quote_pair;
        assert_eq!(strip("\"react\""), "react");
        assert_eq!(strip("\"\"x\"\""), "\"x\"");
        assert_eq!(strip("'a'"), "a");
    }
}
