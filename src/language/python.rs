use crate::language::DefaultVisibility;
use crate::language::LangId;
use crate::language::pack::define_language_pack;
use std::path::{Path, PathBuf};

fn resolve_python_import(raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
    use crate::language::import_resolver::python_candidate_paths;
    let (init_path, module_path) = python_candidate_paths(raw, source_dir, project_root)?;
    if init_path.is_file() {
        return Some(init_path);
    }
    module_path.is_file().then_some(module_path)
}

/// Join the imported name onto a dots-only relative specifier.
///
/// `from . import util` imports the submodule `util` of the current package.
/// Tree-sitter captures the specifier and the name separately, so the module
/// path is `.util`.
pub(crate) fn normalize_relative_specifier<'a>(
    specifier: &'a str,
    symbol: Option<&str>,
    star: bool,
) -> std::borrow::Cow<'a, str> {
    let specifier = specifier.trim();
    if star || specifier.is_empty() || !specifier.bytes().all(|byte| byte == b'.') {
        return std::borrow::Cow::Borrowed(specifier);
    }
    match symbol {
        Some(name) if !name.is_empty() => std::borrow::Cow::Owned(format!("{specifier}{name}")),
        _ => std::borrow::Cow::Borrowed(specifier),
    }
}

const PYTHON_IMPORT_QUERY_STR: &str = r#"
(import_statement
  (dotted_name) @import.path)
(import_statement
  (aliased_import
    name: (dotted_name) @import.path
    alias: (identifier) @import.alias))
(import_from_statement
  module_name: [
    (dotted_name)
    (relative_import)
  ] @import.path
  name: (dotted_name) @import.symbol)
(import_from_statement
  module_name: [
    (dotted_name)
    (relative_import)
  ] @import.path
  (aliased_import name: (_) @import.symbol alias: (identifier) @import.alias))
(import_from_statement
  module_name: [
    (dotted_name)
    (relative_import)
  ] @import.path
  (wildcard_import) @import.star)
"#;

const PYTHON_REFERENCE_QUERY_STR: &str = r#"
(call
  function: (identifier) @reference.name)
(call
  function: (attribute
    attribute: (identifier) @reference.name))
"#;

define_language_pack!(
    spec: PYTHON_SPEC,
    label: "Python",
    lang: LangId::Python,
    grammar: tree_sitter_python::LANGUAGE,
    extensions: ["py", "pyi"],
    resolver: resolve_python_import,
    symbols: {
        static: PYTHON_QUERY,
        accessor: python_query,
        query: r#"
(function_definition
  "async"? @async
  name: (identifier) @name
  parameters: (parameters) @signature
  body: (block (expression_statement (string) @docstring)?)
) @kind.function

(class_definition
  name: (identifier) @name
  body: (block (expression_statement (string) @docstring)?)
) @kind.class

(decorated_definition
  definition: [
    (function_definition
      "async"? @async
      name: (identifier) @name
      parameters: (parameters) @signature
      body: (block (expression_statement (string) @docstring)?)
    ) @kind.function
    (class_definition
      name: (identifier) @name
      body: (block (expression_statement (string) @docstring)?)
    ) @kind.class
  ]
)

(module
  (expression_statement
    (assignment
      left: (identifier) @name) @kind.constant)
)
"#,
    },
    imports_refs: {
        static: PYTHON_IMPORT_REF_QUERY,
        accessor: python_import_ref_query,
        import: PYTHON_IMPORT_QUERY_STR,
        reference: PYTHON_REFERENCE_QUERY_STR,
    },
    dataflow: {
        static: PYTHON_DATAFLOW_QUERY,
        accessor: python_dataflow_query,
        query: r#"
; Assignments
(assignment
  left: (identifier) @def.var)
(augmented_assignment
  left: (identifier) @def.var)
(for_statement
  left: (identifier) @def.var)

; Function parameters
(parameters
  (identifier) @def.param)
(parameters
  (default_parameter
    name: (identifier) @def.param))

; Usages in expression context
(call
  function: (identifier) @use.var)
(argument_list
  (identifier) @use.var)
(binary_operator
  (identifier) @use.var)
(return_statement
  (identifier) @use.var)
(assignment
  right: (identifier) @use.var)
(expression_statement
  (identifier) @use.var)
(attribute
  object: (identifier) @use.var)
(subscript
  value: (identifier) @use.var)
"#,
    },
    import_statement_kinds: ["import_statement", "import_from_statement"],
    class_like_parents: ["class_definition"],
    ancestors: [],
    visibility_from_name: None,
    default_visibility: DefaultVisibility::PublicByDefault,
    doc_comment: None,
    fixture: "tests/fixtures/python/simple_functions.py",
    snapshot: python_insta_snapshot,
    tests {
            use crate::model::SymbolKind;


            #[test]
            fn python_grammar_loads() {
                let _ = grammar_for(LangId::Python);
            }

            #[test]
            fn extract_simple_function() {
                let tree = parse(b"def hello(): pass");
                let symbols = extract_symbols_for(LangId::Python, &tree, b"def hello(): pass");
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0].name, "hello");
                assert!(matches!(symbols[0].kind, SymbolKind::Function));
            }

            #[test]
            fn extract_async_function() {
                let tree = parse(b"async def fetch(): pass");
                let symbols = extract_symbols_for(LangId::Python, &tree, b"async def fetch(): pass");
                assert_eq!(symbols.len(), 1);
                assert!(symbols[0].is_async);
            }

            #[test]
            fn extract_class_and_methods() {
                let src =
                    "class Foo:\n    def __init__(self):\n        pass\n    def bar(self):\n        pass\n";
                let tree = parse(src.as_bytes());
                let symbols = extract_symbols_for(LangId::Python, &tree, src.as_bytes());
                let foo = symbols.iter().find(|s| s.name == "Foo").unwrap();
                assert!(matches!(foo.kind, SymbolKind::Class));
                let bar = symbols.iter().find(|s| s.name == "bar").unwrap();
                assert!(matches!(bar.kind, SymbolKind::Method));
            }

            #[test]
            fn extract_decorated_function() {
                let src = "@decorator\ndef decorated_func(x):\n    return x * 2\n";
                let tree = parse(src.as_bytes());
                let symbols = extract_symbols_for(LangId::Python, &tree, src.as_bytes());
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0].name, "decorated_func");
            }

            #[test]
            fn extract_function_with_docstring() {
                let src = br#"def greet():
            """Say hello."""
            pass
        "#;
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Python, &tree, src);
                assert_eq!(symbols.len(), 1);
                assert_eq!(symbols[0].docstring.as_deref(), Some("Say hello."));
            }

            #[test]
            fn docstring_does_not_eat_content_starting_with_quote() {
                let src = br#"def f():
            """"Quoted" at start."""
            pass
        "#;
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Python, &tree, src);
                assert_eq!(symbols.len(), 1);
                let ds = symbols[0].docstring.as_deref().unwrap();
                assert!(
                    ds.starts_with('"'),
                    "docstring content should start with a quote char, got: {ds:?}"
                );
                assert!(
                    ds.contains("Quoted"),
                    "docstring should contain 'Quoted', got: {ds:?}"
                );
            }

            #[test]
            fn docstring_does_not_eat_content_ending_with_quote() {
                let src = br#"def f():
            '''She said "hello"'''
            pass
        "#;
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Python, &tree, src);
                assert_eq!(symbols.len(), 1);
                let ds = symbols[0].docstring.as_deref().unwrap();
                assert!(
                    ds.contains("hello"),
                    "docstring should contain 'hello', got: {ds:?}"
                );
                assert!(
                    ds.contains(r#"""#),
                    "docstring should preserve inner double quotes, got: {ds:?}"
                );
            }

            #[test]
            fn docstring_multiline_strips_delimiters_not_content() {
                let src = br#"def f():
            """
            She said "hi".
            """
            pass
        "#;
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Python, &tree, src);
                assert_eq!(symbols.len(), 1);
                let ds = symbols[0].docstring.as_deref().unwrap();
                assert!(
                    !ds.starts_with('"'),
                    "docstring should not start with delimiter quote, got: {ds:?}"
                );
                assert!(
                    ds.contains(r#""hi""#),
                    "docstring should preserve inner quotes, got: {ds:?}"
                );
            }


            #[cfg(feature = "dataflow")]
            #[test]
            fn python_dataflow_extracts_assignments_and_parameters() {
                let src = b"def add(x, y):\n    result = x + y\n    return result\n";
                let tree = parse(src);
                let id_gen = crate::model::IdGenerator::new();
                let (nodes, edges) = super::extract_python_dataflow(&tree, src, &id_gen);

                let names: Vec<Option<&str>> = nodes.iter().map(|n| n.name.as_deref()).collect();
                assert!(names.contains(&Some("x")));
                assert!(names.contains(&Some("y")));
                assert!(names.contains(&Some("result")));

                assert!(!edges.is_empty(), "should extract def-use flow edges");
                assert_eq!(edges[0].kind, crate::model::FlowKind::DefUse);
            }

            fn symbol_names(symbols: &[crate::language::RawSymbol<'_>]) -> Vec<String> {
                symbols.iter().map(|s| s.name.to_string()).collect()
            }

            #[test]
            fn module_level_assignment_is_a_constant() {
                let src = b"MAX_RETRIES = 3\n";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Python, &tree, src);
                let found = symbols.iter().find(|s| s.name == "MAX_RETRIES");
                assert!(
                    found.is_some(),
                    "missing MAX_RETRIES: {:?}",
                    symbol_names(&symbols)
                );
                assert!(matches!(found.unwrap().kind, SymbolKind::Constant));
            }

            #[test]
            fn function_local_assignment_is_not_a_symbol() {
                let src = b"def f():\n    local = 1\n";
                let tree = parse(src);
                let symbols = extract_symbols_for(LangId::Python, &tree, src);
                assert!(!symbols.iter().any(|s| s.name == "local"));
            }

            #[test]
            fn relative_import_with_bare_dot_is_captured() {
                use crate::language::extract_imports_and_references_for;
                let src = b"from . import util\n";
                let tree = parse(src);
                let (imports, _, _) = extract_imports_and_references_for(
                    LangId::Python,
                    &tree,
                    src,
                    std::path::Path::new("pkg/mod.py"),
                );
                assert_eq!(imports.len(), 1, "imports: {imports:?}");
                assert_eq!(imports[0].import_specifier, ".");
                assert_eq!(imports[0].symbol.as_deref(), Some("util"));
            }

            #[test]
            fn relative_import_with_module_is_captured() {
                use crate::language::extract_imports_and_references_for;
                let src = b"from .util import helper\n";
                let tree = parse(src);
                let (imports, _, _) = extract_imports_and_references_for(
                    LangId::Python,
                    &tree,
                    src,
                    std::path::Path::new("pkg/mod.py"),
                );
                assert_eq!(imports.len(), 1, "imports: {imports:?}");
                assert_eq!(imports[0].import_specifier, ".util");
                assert_eq!(imports[0].symbol.as_deref(), Some("helper"));
            }

            #[test]
            fn relative_parent_import_is_captured() {
                use crate::language::extract_imports_and_references_for;
                let src = b"from ..pkg.mod import baz\n";
                let tree = parse(src);
                let (imports, _, _) = extract_imports_and_references_for(
                    LangId::Python,
                    &tree,
                    src,
                    std::path::Path::new("pkg/mod.py"),
                );
                assert_eq!(imports.len(), 1, "imports: {imports:?}");
                assert_eq!(imports[0].import_specifier, "..pkg.mod");
            }

            #[test]
            fn aliased_from_import_is_recorded_once() {
                use crate::language::extract_imports_and_references_for;
                let src = b"from a.b import c as d\n";
                let tree = parse(src);
                let (imports, _, _) = extract_imports_and_references_for(
                    LangId::Python,
                    &tree,
                    src,
                    std::path::Path::new("pkg/mod.py"),
                );
                assert_eq!(imports.len(), 1, "imports: {imports:?}");
                assert_eq!(imports[0].symbol.as_deref(), Some("c"));
                assert_eq!(imports[0].alias.as_deref(), Some("d"));
            }

            #[cfg(feature = "dataflow")]
            #[test]
            fn python_dataflow_for_loop_assignment() {
                let src = b"def calc():\n    total = 0\n    for i in items:\n        total += i\n";
                let tree = parse(src);
                let id_gen = crate::model::IdGenerator::new();
                let (nodes, _edges) = super::extract_python_dataflow(&tree, src, &id_gen);

                let names: Vec<Option<&str>> = nodes.iter().map(|n| n.name.as_deref()).collect();
                assert!(names.contains(&Some("total")));
                assert!(names.contains(&Some("i")));
            }
    },
);

// ── Dataflow extraction ─────────────────────────────────────────────

/// Python AST node kinds that introduce a new intra-procedural scope.
#[cfg(feature = "dataflow")]
pub(crate) const PYTHON_FUNCTION_KINDS: &[&str] = &["function_definition"];

/// Extract data nodes and flow edges from a Python parse tree.
#[cfg(feature = "dataflow")]
pub fn extract_python_dataflow(
    tree: &tree_sitter::Tree,
    source: &[u8],
    id_gen: &crate::model::IdGenerator<crate::model::DataNodeId>,
) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
    let Some(query) = python_dataflow_query() else {
        return (Vec::new(), Vec::new());
    };
    crate::language::common::extract_def_use_dataflow(
        tree,
        source,
        query,
        PYTHON_FUNCTION_KINDS,
        id_gen,
    )
}
