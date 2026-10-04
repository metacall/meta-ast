//! What a name means at one call site.
//!
//! A member call counts as MetaCall only when its receiver names the
//! runtime. The receiver text alone is not enough: a local declaration can
//! shadow an alias, and an alias can live in any enclosing scope.
//!
//! [`FileBindings::collect`] indexes every declaration container in one tree
//! walk, and [`FileBindings::resolve`] climbs from the call site outwards and
//! takes the first container that declares the name. A declaration inside a
//! nested block is therefore invisible to a call outside that block, and a
//! parameter list is visible inside the function that owns it.

use std::collections::HashMap;

use tree_sitter::{Node, Tree};

use crate::language::LangId;

/// One name bound in one container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decl {
    /// Names the MetaCall module: `mc` for `import metacall as mc`.
    Module,
    /// Renames one entry point: `m` for `import { metacall as m }`.
    Member(String),
    /// Every name of one module: `from metacall import *`.
    Star { owned: bool },
    /// Binds the name to something else, so it shadows a runtime name.
    Other,
}

/// Names bound in one file, grouped by the container that binds them.
#[derive(Debug, Default)]
pub(crate) struct FileBindings {
    containers: HashMap<usize, HashMap<String, Decl>>,
}

impl FileBindings {
    /// Index every declaration container in one tree walk.
    pub(crate) fn collect(id: LangId, tree: &Tree, source: &[u8]) -> Self {
        // C and Ruby bind no module names, and a file that never spells
        // `metacall` cannot bind one: every specifier and every raw receiver
        // fallback contains the word. Skipping the walk keeps the common file
        // free of both the traversal and its allocations.
        if matches!(id, LangId::C | LangId::Ruby) || !mentions_metacall(source) {
            return Self::default();
        }
        let mut containers: HashMap<usize, HashMap<String, Decl>> = HashMap::new();
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            let mut bound = HashMap::new();
            declarations(id, node, source, &mut bound);
            if !bound.is_empty() {
                containers.insert(node.id(), bound);
            }
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                stack.push(child);
            }
        }
        Self { containers }
    }

    /// The innermost declaration of `name` around `node`.
    ///
    /// Returns `None` when no enclosing container declares the name, which
    /// leaves the caller to fall back to the raw source text. A star import
    /// resolves to the queried name itself.
    pub(crate) fn resolve(&self, node: Node<'_>, name: &str) -> Option<Decl> {
        let mut current = node;
        while let Some(ancestor) = current.parent() {
            if let Some(bound) = self.containers.get(&ancestor.id()) {
                if let Some(decl) = bound.get(name) {
                    return Some(decl.clone());
                }
                if let Some(Decl::Star { owned }) =
                    bound.values().find(|d| matches!(d, Decl::Star { .. }))
                {
                    return Some(if *owned {
                        Decl::Member(name.to_string())
                    } else {
                        Decl::Other
                    });
                }
            }
            current = ancestor;
        }
        None
    }
}

/// True when the source can name the runtime at all.
fn mentions_metacall(source: &[u8]) -> bool {
    const NEEDLE: &[u8] = b"metacall";
    source.windows(NEEDLE.len()).any(|window| window == NEEDLE)
}

/// Every name one container binds, from its direct children.
fn declarations(id: LangId, container: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    match id {
        LangId::JavaScript | LangId::TypeScript | LangId::Tsx => {
            js_declarations(container, source, out)
        }
        LangId::Python => python_declarations(container, source, out),
        LangId::Go => go_declarations(container, source, out),
        LangId::Rust => rust_declarations(container, source, out),
        LangId::Cpp => cpp_declarations(container, source, out),
        LangId::C | LangId::Ruby => {}
    }
}

/// True when the node kind can spell a plain name in a receiver position.
pub(crate) fn is_name_node(kind: &str) -> bool {
    kind.ends_with("identifier") || kind == "constant"
}

/// True for the module specifier and its package subpaths.
///
/// The dot form is not accepted: `metacall.js` is a file, not the runtime.
pub(crate) fn is_metacall_specifier(specifier: &str) -> bool {
    specifier == "metacall" || specifier.starts_with("metacall/")
}

/// True when the dotted module path starts at the MetaCall package.
fn is_metacall_module(module: &str) -> bool {
    module.split('.').next() == Some("metacall")
}

/// True when a Go import path names the MetaCall project.
///
/// Go imports name a module path, not a package name, so the organization
/// segment counts: `github.com/metacall/core/client/go`.
fn is_metacall_go_path(path: &str) -> bool {
    is_metacall_specifier(path) || path.split('/').any(|segment| segment == "metacall")
}

pub(crate) fn text<'a>(node: Node<'_>, source: &'a [u8]) -> &'a str {
    match std::str::from_utf8(&source[node.byte_range()]) {
        Ok(text) => text,
        Err(_) => {
            tracing::warn!(
                start = node.start_byte(),
                end = node.end_byte(),
                "node text is not valid UTF-8, treating as empty"
            );
            ""
        }
    }
}

/// One pair of surrounding quotes, when present.
pub(crate) fn strip_quote_pair(value: &str) -> &str {
    let value = value.strip_prefix(['"', '\'', '`']).unwrap_or(value);
    value.strip_suffix(['"', '\'', '`']).unwrap_or(value)
}

/// Static string text of a node, when the node is a plain string literal.
pub(crate) fn string_text<'a>(node: Node<'_>, source: &'a [u8]) -> Option<&'a str> {
    let kind = node.kind();
    if !(kind.contains("string") || kind == "string_literal") {
        return None;
    }
    let mut cursor = node.walk();
    let interpolated = node
        .children(&mut cursor)
        .any(|child| child.kind() == "interpolation" || child.kind() == "template_substitution");
    (!interpolated).then(|| strip_quote_pair(text(node, source)))
}

/// Innermost value, peeling the wrappers that keep it: parentheses, the
/// TypeScript non-null assertion, `as`, `satisfies`, a type assertion, and the
/// Ruby statement group. A wrapper with no expression is left as it is.
pub(crate) fn unwrap_value(node: Node<'_>) -> Node<'_> {
    let mut current = node;
    loop {
        let inner = match current.kind() {
            // One expression, so the first named child is the value.
            "parenthesized_expression"
            | "non_null_expression"
            | "as_expression"
            | "satisfies_expression" => current.named_child(0),
            // `(a; b)` evaluates to its last statement.
            "parenthesized_statements" | "type_assertion" => last_named_child(current),
            _ => return current,
        };
        match inner {
            Some(inner) => current = inner,
            None => return current,
        }
    }
}

/// Last named child; `type_assertion` holds its type first and its value last,
/// and a Ruby statement group evaluates its last statement.
fn last_named_child(node: Node<'_>) -> Option<Node<'_>> {
    let count = node.named_child_count();
    if count == 0 {
        return None;
    }
    node.named_child(count as u32 - 1)
}

/// True for `require("metacall")`, the inline receiver form.
pub(crate) fn is_require_metacall(node: Node<'_>, source: &[u8]) -> bool {
    if node.kind() != "call_expression" {
        return false;
    }
    let Some(function) = node.child_by_field_name("function") else {
        return false;
    };
    if function.kind() != "identifier" || text(function, source) != "require" {
        return false;
    }
    let Some(arguments) = node.child_by_field_name("arguments") else {
        return false;
    };
    let mut cursor = arguments.walk();
    let mut arguments = arguments.named_children(&mut cursor);
    let Some(first) = arguments.next() else {
        return false;
    };
    if arguments.next().is_some() {
        return false;
    }
    string_text(first, source).is_some_and(is_metacall_specifier)
}

/// Every name a binding pattern introduces.
fn pattern_names(node: Node<'_>, source: &[u8], out: &mut Vec<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" | "constant" => {
            out.push(text(node, source).to_string());
            return;
        }
        // A parameter carries its type in a sibling field, which binds nothing.
        "required_parameter" | "optional_parameter" | "parameter" | "typed_parameter" => {
            if let Some(inner) = node
                .child_by_field_name("pattern")
                .or_else(|| node.named_child(0))
            {
                pattern_names(inner, source, out);
            }
            return;
        }
        "default_parameter" | "typed_default_parameter" => {
            if let Some(inner) = node.child_by_field_name("name") {
                pattern_names(inner, source, out);
            }
            return;
        }
        // A default value is evaluated, not bound.
        "assignment_pattern" => {
            if let Some(inner) = node.child_by_field_name("left") {
                pattern_names(inner, source, out);
            }
            return;
        }
        "pair_pattern" => {
            if let Some(inner) = node.child_by_field_name("value") {
                pattern_names(inner, source, out);
            }
            return;
        }
        _ => {}
    }
    // Everything else is a pattern container: tuple, list, object, rest.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        pattern_names(child, source, out);
    }
}

/// Bind every name of a pattern to `Other`.
fn bind_pattern(node: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let mut names = Vec::new();
    pattern_names(node, source, &mut names);
    for name in names {
        out.insert(name, Decl::Other);
    }
}

/// Bind the `name` field of a declaration to `Other`.
fn bind_name_field(node: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    if let Some(name) = node.child_by_field_name("name") {
        out.insert(text(name, source).to_string(), Decl::Other);
    }
}

/// The declaration a member rename binds, or a plain shadow.
fn member_decl(owned: bool, member: &str) -> Decl {
    if owned {
        Decl::Member(member.to_string())
    } else {
        Decl::Other
    }
}

fn module_decl(owned: bool) -> Decl {
    if owned { Decl::Module } else { Decl::Other }
}

fn js_declarations(container: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let mut cursor = container.walk();
    for child in container.named_children(&mut cursor) {
        js_declare(child, source, out);
    }
}

/// One declaration-bearing node, or the declaration an `export` wraps.
fn js_declare(node: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    match node.kind() {
        "import_statement" => js_import_declarations(node, source, out),
        "lexical_declaration" | "variable_declaration" => {
            js_variable_declarations(node, source, out)
        }
        // `export const mc = ...` and `declare const mc = ...` bind in the
        // enclosing container, not in the wrapper.
        "export_statement" | "ambient_declaration" => {
            let mut cursor = node.walk();
            for wrapped in node.named_children(&mut cursor) {
                js_declare(wrapped, source, out);
            }
        }
        "function_declaration"
        | "function_expression"
        | "generator_function_declaration"
        | "generator_function"
        | "class_declaration"
        | "class_expression"
        | "method_definition"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => bind_name_field(node, source, out),
        "formal_parameters" => {
            let mut cursor = node.walk();
            for param in node.named_children(&mut cursor) {
                bind_pattern(param, source, out);
            }
        }
        // A single-parameter arrow binds its parameter directly.
        "arrow_function" => {
            if let Some(param) = node.child_by_field_name("parameter") {
                bind_pattern(param, source, out);
            }
        }
        "for_in_statement" | "for_of_statement" => {
            if let Some(left) = node.child_by_field_name("left") {
                match left.kind() {
                    "lexical_declaration" | "variable_declaration" => {
                        js_variable_declarations(left, source, out)
                    }
                    _ => bind_pattern(left, source, out),
                }
            }
        }
        "catch_clause" => {
            if let Some(param) = node.child_by_field_name("parameter") {
                bind_pattern(param, source, out);
            }
        }
        _ => {}
    }
}

fn js_import_declarations(node: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let Some(specifier) = node.child_by_field_name("source") else {
        return;
    };
    let owned = string_text(specifier, source).is_some_and(is_metacall_specifier);
    let mut cursor = node.walk();
    for clause in node.named_children(&mut cursor) {
        if clause.kind() != "import_clause" {
            continue;
        }
        let mut clause_cursor = clause.walk();
        for child in clause.named_children(&mut clause_cursor) {
            match child.kind() {
                // A default or namespace import binds the module itself.
                "identifier" | "namespace_import" => {
                    let local = if child.kind() == "namespace_import" {
                        child.named_child(0)
                    } else {
                        Some(child)
                    };
                    if let Some(local) = local {
                        out.insert(text(local, source).to_string(), module_decl(owned));
                    }
                }
                "named_imports" => {
                    let mut named_cursor = child.walk();
                    for specifier in child.named_children(&mut named_cursor) {
                        if specifier.kind() != "import_specifier" {
                            continue;
                        }
                        let Some(member) = specifier.child_by_field_name("name") else {
                            continue;
                        };
                        let local = specifier.child_by_field_name("alias").unwrap_or(member);
                        let member = strip_quote_pair(text(member, source));
                        out.insert(text(local, source).to_string(), member_decl(owned, member));
                    }
                }
                _ => {}
            }
        }
    }
}

fn js_variable_declarations(node: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let mut cursor = node.walk();
    for declarator in node.named_children(&mut cursor) {
        if declarator.kind() != "variable_declarator" {
            continue;
        }
        let Some(pattern) = declarator.child_by_field_name("name") else {
            continue;
        };
        let metacall = declarator
            .child_by_field_name("value")
            .map(unwrap_value)
            .is_some_and(|value| is_require_metacall(value, source));
        if !metacall {
            bind_pattern(pattern, source, out);
            continue;
        }
        match pattern.kind() {
            "object_pattern" => {
                let mut entries = pattern.walk();
                for entry in pattern.named_children(&mut entries) {
                    match entry.kind() {
                        "shorthand_property_identifier_pattern" => {
                            let name = text(entry, source).to_string();
                            out.insert(name.clone(), Decl::Member(name));
                        }
                        "pair_pattern" => {
                            let Some(key) = entry.child_by_field_name("key") else {
                                continue;
                            };
                            let member = strip_quote_pair(text(key, source)).to_string();
                            let Some(value) = entry.child_by_field_name("value") else {
                                continue;
                            };
                            let mut names = Vec::new();
                            pattern_names(value, source, &mut names);
                            for name in names {
                                out.insert(name, Decl::Member(member.clone()));
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {
                let mut names = Vec::new();
                pattern_names(pattern, source, &mut names);
                for name in names {
                    out.insert(name, Decl::Module);
                }
            }
        }
    }
}

fn python_declarations(container: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let mut cursor = container.walk();
    for child in container.named_children(&mut cursor) {
        match child.kind() {
            "import_statement" => python_import_declarations(child, source, out),
            "import_from_statement" => python_from_declarations(child, source, out),
            "function_definition" | "class_definition" => bind_name_field(child, source, out),
            "parameters" | "lambda_parameters" => {
                let mut params = child.walk();
                for param in child.named_children(&mut params) {
                    bind_pattern(param, source, out);
                }
            }
            "assignment" | "augmented_assignment" | "named_expression" | "for_statement" => {
                if let Some(left) = child.child_by_field_name("left") {
                    bind_pattern(left, source, out);
                }
            }
            "except_clause" => {
                if let Some(alias) = child.child_by_field_name("name") {
                    bind_pattern(alias, source, out);
                }
            }
            "global_statement" | "nonlocal_statement" => {
                let mut names = child.walk();
                for name in child.named_children(&mut names) {
                    out.insert(text(name, source).to_string(), Decl::Other);
                }
            }
            _ => {}
        }
    }
}

fn python_import_declarations(node: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "aliased_import" => {
                let Some(module) = child.child_by_field_name("name") else {
                    continue;
                };
                let local = child
                    .child_by_field_name("alias")
                    .map(|alias| text(alias, source))
                    .unwrap_or_else(|| leading_segment(text(module, source)));
                let owned = is_metacall_module(text(module, source));
                out.insert(local.to_string(), module_decl(owned));
            }
            // `import metacall.foo` binds the leading package.
            "dotted_name" => {
                let module = text(child, source);
                let owned = is_metacall_module(module);
                out.insert(leading_segment(module).to_string(), module_decl(owned));
            }
            _ => {}
        }
    }
}

fn python_from_declarations(node: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let owned = node
        .child_by_field_name("module_name")
        .is_some_and(|module| is_metacall_module(text(module, source)));
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "aliased_import" => {
                let Some(member) = child.child_by_field_name("name") else {
                    continue;
                };
                let local = child
                    .child_by_field_name("alias")
                    .map(|alias| text(alias, source))
                    .unwrap_or_else(|| leading_segment(text(member, source)));
                out.insert(local.to_string(), member_decl(owned, text(member, source)));
            }
            "dotted_name" => {
                let member = text(child, source);
                out.insert(
                    leading_segment(member).to_string(),
                    member_decl(owned, member),
                );
            }
            // `from metacall import *` exposes every entry point.
            "wildcard_import" => {
                out.insert(String::new(), Decl::Star { owned });
            }
            _ => {}
        }
    }
}

fn leading_segment(value: &str) -> &str {
    value.split('.').next().unwrap_or(value)
}

fn go_declarations(container: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let mut cursor = container.walk();
    for child in container.named_children(&mut cursor) {
        match child.kind() {
            "import_declaration" => {
                for spec in spec_children(child) {
                    if spec.kind() != "import_spec" {
                        continue;
                    }
                    let Some(path) = spec
                        .child_by_field_name("path")
                        .and_then(|path| string_text(path, source))
                    else {
                        continue;
                    };
                    let owned = is_metacall_go_path(path);
                    let alias = spec.child_by_field_name("name");
                    match alias.map(|alias| text(alias, source)) {
                        // A dot import exposes every name in the package.
                        Some(".") => {
                            out.insert(String::new(), Decl::Star { owned });
                        }
                        Some(local) => {
                            out.insert(local.to_string(), module_decl(owned));
                        }
                        None => {
                            out.insert(package_name(path).to_string(), module_decl(owned));
                        }
                    }
                }
            }
            "var_declaration" | "const_declaration" => {
                for spec in spec_children(child) {
                    if !matches!(spec.kind(), "var_spec" | "const_spec") {
                        continue;
                    }
                    let mut names = spec.walk();
                    for name in spec.children_by_field_name("name", &mut names) {
                        out.insert(text(name, source).to_string(), Decl::Other);
                    }
                }
            }
            // `x := value` binds every identifier on its left.
            "short_var_declaration" => {
                if let Some(left) = child.child_by_field_name("left") {
                    bind_pattern(left, source, out);
                }
            }
            "function_declaration" | "method_declaration" | "type_declaration" | "type_alias" => {
                bind_name_field(child, source, out)
            }
            "parameter_list" => {
                let mut params = child.walk();
                for param in child.named_children(&mut params) {
                    bind_pattern(param, source, out);
                }
            }
            _ => {}
        }
    }
}

/// Direct spec children, unwrapping the one list node a parenthesized
/// declaration adds.
fn spec_children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut specs = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind().ends_with("_list") {
            let mut list_cursor = child.walk();
            specs.extend(child.named_children(&mut list_cursor));
        } else {
            specs.push(child);
        }
    }
    specs
}

fn package_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn rust_declarations(container: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let mut cursor = container.walk();
    for child in container.named_children(&mut cursor) {
        match child.kind() {
            "use_declaration" => {
                if let Some(argument) = child.child_by_field_name("argument") {
                    rust_use_declarations(argument, source, out);
                }
            }
            "let_declaration" => {
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    bind_pattern(pattern, source, out);
                }
            }
            "function_item" | "struct_item" | "enum_item" | "union_item" | "trait_item"
            | "mod_item" | "static_item" | "const_item" | "type_item" | "macro_definition" => {
                bind_name_field(child, source, out)
            }
            "parameters" => {
                let mut params = child.walk();
                for param in child.named_children(&mut params) {
                    bind_pattern(param, source, out);
                }
            }
            "closure_expression" => {
                if let Some(params) = child.child_by_field_name("parameters") {
                    let mut params_cursor = params.walk();
                    for param in params.named_children(&mut params_cursor) {
                        bind_pattern(param, source, out);
                    }
                }
            }
            _ => {}
        }
    }
}

/// `use metacall as mc`, `use metacall::load_from_file as load`, and the bare
/// `use metacall::metacall` all bind a local name.
fn rust_use_declarations(argument: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    if argument.kind() == "use_as_clause" {
        let Some(path) = argument.child_by_field_name("path") else {
            return;
        };
        let Some(alias) = argument.child_by_field_name("alias") else {
            return;
        };
        let owned =
            leftmost_identifier(path).is_some_and(|root| is_metacall_specifier(text(root, source)));
        let decl = match path.kind() {
            "identifier" => module_decl(owned),
            _ => path
                .child_by_field_name("name")
                .map(|member| member_decl(owned, text(member, source)))
                .unwrap_or(Decl::Other),
        };
        out.insert(text(alias, source).to_string(), decl);
        return;
    }
    let owned =
        leftmost_identifier(argument).is_some_and(|root| is_metacall_specifier(text(root, source)));
    match argument.kind() {
        "identifier" => {
            out.insert(text(argument, source).to_string(), module_decl(owned));
        }
        "scoped_identifier" => {
            if let Some(member) = argument.child_by_field_name("name") {
                out.insert(
                    text(member, source).to_string(),
                    member_decl(owned, text(member, source)),
                );
            }
        }
        _ => {}
    }
}

/// The innermost `path` of a Rust scoped path: its first segment.
fn leftmost_identifier(path: Node<'_>) -> Option<Node<'_>> {
    match path.child_by_field_name("path") {
        Some(inner) => leftmost_identifier(inner),
        None => (path.kind() == "identifier").then_some(path),
    }
}

fn cpp_declarations(container: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    let mut cursor = container.walk();
    for child in container.named_children(&mut cursor) {
        match child.kind() {
            "namespace_alias_definition" => {
                let Some(alias) = child.child_by_field_name("name") else {
                    continue;
                };
                let mut alias_cursor = child.walk();
                let target = child
                    .named_children(&mut alias_cursor)
                    .find(|node| node.id() != alias.id());
                let owned =
                    target.is_some_and(|target| is_metacall_specifier(text(target, source)));
                out.insert(text(alias, source).to_string(), module_decl(owned));
            }
            "using_declaration" => {
                let mut path_cursor = child.walk();
                let Some(path) = child
                    .named_children(&mut path_cursor)
                    .find(|node| node.kind() == "scoped_identifier")
                else {
                    continue;
                };
                let Some(member) = path.child_by_field_name("name") else {
                    continue;
                };
                let owned = leftmost_identifier(path)
                    .is_some_and(|root| is_metacall_specifier(text(root, source)));
                out.insert(
                    text(member, source).to_string(),
                    member_decl(owned, text(member, source)),
                );
            }
            "function_definition"
            | "class_specifier"
            | "struct_specifier"
            | "namespace_definition"
            | "alias_declaration" => bind_name_field(child, source, out),
            "declaration" => {
                let mut declarators = child.walk();
                for declarator in child.children_by_field_name("declarator", &mut declarators) {
                    declarator_names(declarator, source, out);
                }
            }
            "parameter_list" => {
                let mut params = child.walk();
                for param in child.named_children(&mut params) {
                    declarator_names(param, source, out);
                }
            }
            _ => {}
        }
    }
}

/// Every name a C++ declarator introduces.
fn declarator_names(node: Node<'_>, source: &[u8], out: &mut HashMap<String, Decl>) {
    if is_name_node(node.kind()) {
        out.insert(text(node, source).to_string(), Decl::Other);
        return;
    }
    match node.kind() {
        "init_declarator"
        | "parameter_declaration"
        | "pointer_declarator"
        | "reference_declarator"
        | "array_declarator" => {
            if let Some(inner) = node.child_by_field_name("declarator") {
                declarator_names(inner, source, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::grammar_for;

    fn parse(id: LangId, source: &[u8]) -> tree_sitter::Tree {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar_for(id)).unwrap();
        parser.parse(source, None).unwrap()
    }

    /// Classify `name` at the receiver of the first member call of `source`.
    fn receiver_decl(id: LangId, source: &[u8], name: &str) -> Option<Decl> {
        let tree = parse(id, source);
        let bindings = FileBindings::collect(id, &tree, source);
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            let callee = node.child_by_field_name("function");
            let receiver = callee.and_then(|callee| match callee.kind() {
                "member_expression" | "attribute" => callee.child_by_field_name("object"),
                "selector_expression" => callee.child_by_field_name("operand"),
                "scoped_identifier" => callee.child_by_field_name("path"),
                _ => None,
            });
            if let Some(receiver) = receiver {
                return bindings.resolve(unwrap_value(receiver), name);
            }
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                stack.push(child);
            }
        }
        None
    }

    #[test]
    fn an_exported_require_alias_is_the_module() {
        let source = b"export const mc = require(\"metacall\");\nmc.metacall(\"sum\");";
        assert_eq!(
            receiver_decl(LangId::TypeScript, source, "mc"),
            Some(Decl::Module),
            "an exported declaration is still a declaration"
        );
    }

    #[test]
    fn a_parenthesized_require_alias_is_the_module() {
        let source = b"const mc = (require(\"metacall\"));\nmc.metacall(\"sum\");";
        assert_eq!(
            receiver_decl(LangId::JavaScript, source, "mc"),
            Some(Decl::Module)
        );
    }

    #[test]
    fn an_alias_inside_a_function_is_the_module() {
        let source =
            b"function run() { const mc = require(\"metacall\"); mc.metacall(\"sum\"); }\nrun();";
        assert_eq!(
            receiver_decl(LangId::JavaScript, source, "mc"),
            Some(Decl::Module)
        );
    }

    #[test]
    fn a_local_declaration_shadows_the_alias() {
        let source = b"const mc = require(\"metacall\");\nfunction run() {\n  const mc = helper();\n  mc.metacall(\"sum\");\n}";
        assert_eq!(
            receiver_decl(LangId::JavaScript, source, "mc"),
            Some(Decl::Other),
            "the innermost declaration wins"
        );
    }

    #[test]
    fn a_name_imported_from_another_module_is_not_the_runtime() {
        let source = b"import { metacall as m } from \"metacall-client\";\nm.metacall(\"sum\");";
        assert_eq!(
            receiver_decl(LangId::JavaScript, source, "m"),
            Some(Decl::Other)
        );
    }

    #[test]
    fn a_python_import_inside_a_function_is_the_module() {
        let source = b"def run():\n    import metacall as mc\n    mc.metacall(\"sum\")\n";
        assert_eq!(
            receiver_decl(LangId::Python, source, "mc"),
            Some(Decl::Module)
        );
    }

    #[test]
    fn a_go_module_path_from_the_project_is_the_runtime() {
        let source = b"import metacall \"github.com/metacall/core/client/go\"\n\nfunc run() { metacall.LoadFromFile(\"py\", scripts) }";
        assert_eq!(
            receiver_decl(LangId::Go, source, "metacall"),
            Some(Decl::Module),
            "the Go client is imported by module path"
        );
    }

    #[test]
    fn a_dot_specifier_is_a_file_not_the_module() {
        assert!(!is_metacall_specifier("metacall.js"));
        assert!(is_metacall_specifier("metacall"));
        assert!(is_metacall_specifier("metacall/client"));
    }
}
