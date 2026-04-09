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

pub fn to_inspect_json(symbols: &[Symbol]) -> Result<String, serde_json::Error> {
    let output = symbols_to_inspect_output(symbols);
    serde_json::to_string_pretty(&output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::language::LangId;
    use crate::model::{LineColumn, SourceRange, SymbolId, SymbolKind};

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
        let json = to_inspect_json(&[]).unwrap();
        assert_eq!(
            json,
            "{\n  \"funcs\": [],\n  \"classes\": [],\n  \"objects\": []\n}"
        );
    }

    #[test]
    fn symbol_to_func_entry() {
        let sym = make_symbol(1, "my_func", SymbolKind::Function);
        let output = symbols_to_inspect_output(&[sym]);
        assert_eq!(output.funcs.len(), 1);
        assert_eq!(output.classes.len(), 0);
        assert_eq!(output.objects.len(), 0);
        assert_eq!(output.funcs[0].name, "my_func");
    }

    #[test]
    fn symbol_to_class_entry() {
        let sym = make_symbol(1, "MyClass", SymbolKind::Class);
        let output = symbols_to_inspect_output(&[sym]);
        assert_eq!(output.classes.len(), 1);
        assert_eq!(output.funcs.len(), 0);
        assert_eq!(output.objects.len(), 0);
        assert_eq!(output.classes[0].name, "MyClass");
    }

    #[test]
    fn symbol_to_object_entry() {
        let sym = make_symbol(1, "MY_CONST", SymbolKind::Constant);
        let output = symbols_to_inspect_output(&[sym]);
        assert_eq!(output.objects.len(), 1);
        assert_eq!(output.funcs.len(), 0);
        assert_eq!(output.classes.len(), 0);
        assert_eq!(output.objects[0].name, "MY_CONST");
    }

    #[test]
    fn method_maps_to_func() {
        let sym = make_symbol(1, "do_thing", SymbolKind::Method);
        let output = symbols_to_inspect_output(&[sym]);
        assert_eq!(output.funcs.len(), 1);
        assert_eq!(output.funcs[0].name, "do_thing");
    }

    #[test]
    fn struct_maps_to_class() {
        let sym = make_symbol(1, "Point", SymbolKind::Struct);
        let output = symbols_to_inspect_output(&[sym]);
        assert_eq!(output.classes.len(), 1);
        assert_eq!(output.classes[0].name, "Point");
    }

    #[test]
    fn to_inspect_json_valid() {
        let symbols = vec![
            make_symbol(1, "f1", SymbolKind::Function),
            make_symbol(2, "C1", SymbolKind::Class),
            make_symbol(3, "OBJ", SymbolKind::Constant),
        ];
        let json = to_inspect_json(&symbols).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_object());
        assert!(parsed["funcs"].is_array());
        assert!(parsed["classes"].is_array());
        assert!(parsed["objects"].is_array());
    }

    #[test]
    fn inspect_output_required_keys() {
        let json = to_inspect_json(&[]).unwrap();
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
