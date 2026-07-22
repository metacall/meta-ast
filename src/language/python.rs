use crate::language::{DefaultVisibility, LanguageSpec};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

fn resolve_python_import(raw: &str, source_dir: &Path, project_root: &Path) -> Option<PathBuf> {
    let raw = raw.trim_matches(|c| c == '"' || c == '\'');
    if raw.is_empty() {
        return None;
    }

    if raw.starts_with('.') {
        let relative = raw.trim_start_matches('.');
        if relative.is_empty() {
            return Some(source_dir.join("__init__.py"));
        }
        let path = source_dir.join(relative.replace('.', std::path::MAIN_SEPARATOR_STR));
        if path.join("__init__.py").exists() {
            return Some(path.join("__init__.py"));
        }
        Some(path.with_extension("py"))
    } else {
        let path = project_root.join(raw.replace('.', std::path::MAIN_SEPARATOR_STR));
        if path.join("__init__.py").exists() {
            return Some(path.join("__init__.py"));
        }
        Some(path.with_extension("py"))
    }
}

static PYTHON_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_python::LANGUAGE.into(),
        r#"
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
"#,
        "Python",
    )
});

fn python_query() -> &'static tree_sitter::Query {
    &PYTHON_QUERY
}

const PYTHON_IMPORT_QUERY_STR: &str = r#"
(import_statement
  (dotted_name) @import.path)
(import_statement
  (aliased_import
    name: (dotted_name) @import.path
    alias: (identifier) @import.alias))
(import_from_statement
  module_name: (dotted_name) @import.path
  name: (_) @import.symbol)
(import_from_statement
  module_name: (dotted_name) @import.path
  (aliased_import name: (_) @import.symbol alias: (identifier) @import.alias))
(import_from_statement
  module_name: (dotted_name) @import.path
  (wildcard_import) @import.star)
"#;

const PYTHON_REFERENCE_QUERY_STR: &str = r#"
(call
  function: (identifier) @reference.name)
(call
  function: (attribute
    attribute: (identifier) @reference.name))
"#;

static PYTHON_IMPORT_REF_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_python::LANGUAGE.into(),
        &format!(
            "{}\n{}",
            PYTHON_IMPORT_QUERY_STR, PYTHON_REFERENCE_QUERY_STR
        ),
        "Python combined import+ref",
    )
});

fn python_import_ref_query() -> &'static tree_sitter::Query {
    &PYTHON_IMPORT_REF_QUERY
}

pub(crate) const PYTHON_SPEC: LanguageSpec = LanguageSpec {
    extensions: &["py", "pyi"],
    grammar_fn: || tree_sitter_python::LANGUAGE.into(),
    query_fn: python_query,
    import_path_resolver: resolve_python_import,
    import_ref_query_fn: python_import_ref_query,
    class_like_parents: &["class_definition"],
    ancestor_visibility_rules: &[],
    visibility_from_name: None,
    import_statement_kinds: &["import_statement", "import_from_statement"],
    default_visibility: DefaultVisibility::PublicByDefault,
    doc_comment_config: None,
};

// ── Dataflow extraction ─────────────────────────────────────────────

#[cfg(feature = "dataflow")]
static PYTHON_DATAFLOW_QUERY: LazyLock<tree_sitter::Query> = LazyLock::new(|| {
    crate::language::common::compile_query(
        &tree_sitter_python::LANGUAGE.into(),
        r#"
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
        "Python dataflow",
    )
});

/// Extract data nodes and flow edges from a Python parse tree.
#[cfg(feature = "dataflow")]
pub fn extract_python_dataflow(
    tree: &tree_sitter::Tree,
    source: &[u8],
    id_gen: &crate::model::IdGenerator<crate::model::DataNodeId>,
) -> (Vec<crate::model::DataNode>, Vec<crate::model::FlowEdge>) {
    use crate::model::{DataNode, DataNodeId, DataScope, FlowEdge, FlowKind};
    use tree_sitter::StreamingIterator;

    let query = &*PYTHON_DATAFLOW_QUERY;
    let mut cursor = tree_sitter::QueryCursor::new();

    let mut defs: Vec<(String, usize, tree_sitter::Node, bool)> = Vec::new();
    let mut uses: Vec<(String, usize, tree_sitter::Node, usize)> = Vec::new();

    let mut matches = cursor.matches(query, tree.root_node(), source);
    while let Some(m) = matches.next() {
        for capture in m.captures {
            let capture_name = query.capture_names()[capture.index as usize];
            let node = capture.node;
            let byte_pos = node.start_byte();
            let name = match node.utf8_text(source) {
                Ok(t) => t.to_string(),
                Err(_) => continue,
            };

            match capture_name {
                "def.var" => {
                    defs.push((name, byte_pos, node, false));
                }
                "def.param" => {
                    defs.push((name, byte_pos, node, true));
                }
                "use.var" => {
                    let func_start = enclosing_function_start(node);
                    uses.push((name, byte_pos, node, func_start));
                }
                _ => {}
            }
        }
    }

    let mut nodes = Vec::new();
    let mut def_ids: Vec<(String, usize, DataNodeId, bool)> = Vec::new();

    for (name, byte_pos, node, is_param) in &defs {
        let scope = if *is_param {
            DataScope::Parameter
        } else {
            DataScope::Local
        };
        let dn = DataNode {
            id: id_gen.next(),
            symbol_id: None,
            name: Some(name.clone()),
            scope,
            type_hint: None,
            source_range: source_range_from_node(node),
        };
        def_ids.push((name.clone(), *byte_pos, dn.id, *is_param));
        nodes.push(dn);
    }

    let mut edges = Vec::new();

    for (use_name, use_pos, use_node, use_func_start) in &uses {
        let mut best_def: Option<&(String, usize, DataNodeId, bool)> = None;

        for def in &def_ids {
            if def.0 == *use_name && def.1 < *use_pos {
                let def_func_start = enclosing_function_start_for_id(def.1, tree);
                if def_func_start == *use_func_start {
                    match best_def {
                        None => best_def = Some(def),
                        Some(best) => {
                            if def.1 > best.1 {
                                best_def = Some(def);
                            }
                        }
                    }
                }
            }
        }

        if let Some(def) = best_def {
            let use_dn = DataNode {
                id: id_gen.next(),
                symbol_id: None,
                name: Some(use_name.clone()),
                scope: DataScope::Local,
                type_hint: None,
                source_range: source_range_from_node(use_node),
            };
            let target_id = use_dn.id;
            nodes.push(use_dn);

            edges.push(FlowEdge {
                source: def.2,
                target: target_id,
                kind: FlowKind::DefUse,
                confidence: 0.9,
            });
        }
    }

    (nodes, edges)
}

#[cfg(feature = "dataflow")]
fn enclosing_function_start(node: tree_sitter::Node) -> usize {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_definition" {
            return parent.start_byte();
        }
        current = parent.parent();
    }
    0
}

#[cfg(feature = "dataflow")]
fn enclosing_function_start_for_id(byte_pos: usize, tree: &tree_sitter::Tree) -> usize {
    let node = tree.root_node();
    find_enclosing_function(node, byte_pos)
}

#[cfg(feature = "dataflow")]
fn find_enclosing_function(node: tree_sitter::Node, byte_pos: usize) -> usize {
    if node.start_byte() <= byte_pos && byte_pos < node.end_byte() {
        if node.kind() == "function_definition" {
            return node.start_byte();
        }
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i as u32) {
                let result = find_enclosing_function(child, byte_pos);
                if result != 0 {
                    return result;
                }
            }
        }
    }
    0
}

#[cfg(feature = "dataflow")]
fn source_range_from_node(node: &tree_sitter::Node) -> crate::model::SourceRange {
    use crate::model::{LineColumn, SourceRange};
    SourceRange {
        byte_start: node.start_byte(),
        byte_end: node.end_byte(),
        start: LineColumn {
            line: node.start_position().row,
            column: node.start_position().column,
        },
        end: LineColumn {
            line: node.end_position().row,
            column: node.end_position().column,
        },
    }
}

#[cfg(test)]
mod tests {
    use crate::language::{LangId, extract_symbols_for, grammar_for};
    use crate::model::SymbolKind;

    fn parse(source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(LangId::Python)).unwrap();
        parser.parse(source, None).unwrap()
    }

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

    #[test]
    fn python_insta_snapshot() {
        let src = std::fs::read_to_string(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/python/simple_functions.py"),
        )
        .unwrap();
        let tree = parse(src.as_bytes());
        let symbols = extract_symbols_for(LangId::Python, &tree, src.as_bytes());
        insta::assert_json_snapshot!(symbols);
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
}
