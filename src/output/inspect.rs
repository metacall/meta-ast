use crate::model::output::InspectOutput;
use crate::model::{Symbol, SymbolKind};

pub fn symbols_to_inspect_output(symbols: &[Symbol]) -> InspectOutput {
    let mut output = InspectOutput {
        funcs: Vec::new(),
        classes: Vec::new(),
        objects: Vec::new(),
    };

    for symbol in symbols {
        match symbol.kind {
            SymbolKind::Function | SymbolKind::Method => {
                output.funcs.push(crate::model::output::FuncEntry {
                    name: symbol.name.clone(),
                    source_range: symbol.source_range.clone(),
                    visibility: symbol.visibility,
                    signature: symbol.signature.clone(),
                    docstring: symbol.docstring.clone(),
                    is_async: symbol.is_async,
                });
            }
            SymbolKind::Class
            | SymbolKind::Struct
            | SymbolKind::Interface
            | SymbolKind::Trait
            | SymbolKind::Enum => {
                output.classes.push(crate::model::output::ClassEntry {
                    name: symbol.name.clone(),
                    source_range: symbol.source_range.clone(),
                    visibility: symbol.visibility,
                    signature: symbol.signature.clone(),
                    docstring: symbol.docstring.clone(),
                });
            }
            SymbolKind::Object
            | SymbolKind::Constant
            | SymbolKind::Static
            | SymbolKind::Module
            | SymbolKind::Namespace
            | SymbolKind::TypeAlias => {
                output.objects.push(crate::model::output::ObjectEntry {
                    name: symbol.name.clone(),
                    source_range: symbol.source_range.clone(),
                    visibility: symbol.visibility,
                    signature: symbol.signature.clone(),
                    docstring: symbol.docstring.clone(),
                });
            }
        }
    }

    output
}

pub fn serialize_inspect(
    symbols: &[Symbol],
    format: &crate::output::OutputFormat,
) -> anyhow::Result<String> {
    let output = symbols_to_inspect_output(symbols);
    format.serialize(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::LangId;
    use crate::model::{LineColumn, SourceRange, SymbolId, SymbolKind};
    use crate::output::OutputFormat;

    fn make_symbol(id: u32, name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            id: SymbolId(id),
            name: name.to_string(),
            kind,
            language: LangId::Python,
            file_path: std::path::PathBuf::from("test.py"),
            source_range: SourceRange {
                byte_start: 0,
                byte_end: 10,
                start: LineColumn { line: 0, column: 0 },
                end: LineColumn {
                    line: 0,
                    column: 10,
                },
            },
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        }
    }

    #[test]
    fn empty_symbols_produces_empty_json() {
        let json = serialize_inspect(&[], &OutputFormat::Json).unwrap();
        assert_eq!(
            json,
            "{\n  \"funcs\": [],\n  \"classes\": [],\n  \"objects\": []\n}"
        );
    }

    #[test]
    fn symbols_map_to_expected_bucket() {
        let cases = [
            ("my_func", SymbolKind::Function, 1usize, 0usize, 0usize),
            ("do_thing", SymbolKind::Method, 1usize, 0usize, 0usize),
            ("MyClass", SymbolKind::Class, 0usize, 1usize, 0usize),
            ("Point", SymbolKind::Struct, 0usize, 1usize, 0usize),
            ("MY_CONST", SymbolKind::Constant, 0usize, 0usize, 1usize),
        ];

        for (name, kind, funcs, classes, objects) in cases {
            let sym = make_symbol(1, name, kind);
            let output = symbols_to_inspect_output(&[sym]);
            assert_eq!(output.funcs.len(), funcs, "unexpected funcs for {name}");
            assert_eq!(
                output.classes.len(),
                classes,
                "unexpected classes for {name}"
            );
            assert_eq!(
                output.objects.len(),
                objects,
                "unexpected objects for {name}"
            );
            if funcs == 1 {
                assert_eq!(output.funcs[0].name, name);
            }
            if classes == 1 {
                assert_eq!(output.classes[0].name, name);
            }
            if objects == 1 {
                assert_eq!(output.objects[0].name, name);
            }
        }
    }

    #[test]
    fn to_inspect_json_valid() {
        let symbols = vec![
            make_symbol(1, "f1", SymbolKind::Function),
            make_symbol(2, "C1", SymbolKind::Class),
            make_symbol(3, "OBJ", SymbolKind::Constant),
        ];
        let json = serialize_inspect(&symbols, &OutputFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_object());
        assert!(parsed["funcs"].is_array());
        assert!(parsed["classes"].is_array());
        assert!(parsed["objects"].is_array());
    }

    #[test]
    fn inspect_output_required_keys() {
        let json = serialize_inspect(&[], &OutputFormat::Json).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let keys: std::collections::HashSet<&str> = parsed
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        let expected: std::collections::HashSet<&str> =
            ["funcs", "classes", "objects"].into_iter().collect();
        assert_eq!(keys, expected);
    }
}
