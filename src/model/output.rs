use serde::Serialize;

use crate::model::{SourceRange, Visibility};

#[derive(Debug, Clone, Serialize)]
pub struct FuncEntry {
    pub name: String,
    pub source_range: SourceRange,
    pub visibility: Option<Visibility>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
    #[serde(rename = "async")]
    pub is_async: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClassEntry {
    pub name: String,
    pub source_range: SourceRange,
    pub visibility: Option<Visibility>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ObjectEntry {
    pub name: String,
    pub source_range: SourceRange,
    pub visibility: Option<Visibility>,
    pub signature: Option<String>,
    pub docstring: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectOutput {
    pub funcs: Vec<FuncEntry>,
    pub classes: Vec<ClassEntry>,
    pub objects: Vec<ObjectEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LineColumn, SourceRange, Visibility};
    use serde_json;

    fn sample_source_range() -> SourceRange {
        SourceRange {
            byte_start: 0,
            byte_end: 5,
            start: LineColumn { line: 1, column: 0 },
            end: LineColumn { line: 1, column: 5 },
        }
    }

    #[test]
    fn inspect_output_empty_serializes() {
        let output = InspectOutput {
            funcs: vec![],
            classes: vec![],
            objects: vec![],
        };
        let json = serde_json::to_string(&output).unwrap();
        assert_eq!(json, "{\"funcs\":[],\"classes\":[],\"objects\":[]}");
    }

    #[test]
    fn func_entry_has_required_keys() {
        let entry = FuncEntry {
            name: "f".into(),
            source_range: sample_source_range(),
            visibility: None,
            signature: None,
            docstring: None,
            is_async: false,
        };
        let val: serde_json::Value = serde_json::to_value(&entry).unwrap();
        let obj = val.as_object().unwrap();
        let expected_keys = [
            "name",
            "source_range",
            "async",
            "visibility",
            "signature",
            "docstring",
        ];
        for key in &expected_keys {
            assert!(obj.contains_key(*key), "missing key: {key}");
        }
    }

    #[test]
    fn func_entry_async_field_renamed() {
        let entry = FuncEntry {
            name: "g".into(),
            source_range: sample_source_range(),
            visibility: None,
            signature: None,
            docstring: None,
            is_async: true,
        };
        let val: serde_json::Value = serde_json::to_value(&entry).unwrap();
        assert_eq!(val["async"], true);
        assert!(val.get("is_async").is_none());
    }

    #[test]
    fn class_entry_serialization() {
        let entry = ClassEntry {
            name: "MyClass".into(),
            source_range: sample_source_range(),
            visibility: Some(Visibility::Public),
            signature: Some("class MyClass".into()),
            docstring: Some("a class".into()),
        };
        let val: serde_json::Value = serde_json::to_value(&entry).unwrap();
        assert_eq!(val["name"], "MyClass");
        assert_eq!(val["visibility"], "Public");
        assert_eq!(val["signature"], "class MyClass");
        assert_eq!(val["docstring"], "a class");
        assert!(val["source_range"].is_object());
    }

    #[test]
    fn object_entry_serialization() {
        let entry = ObjectEntry {
            name: "obj".into(),
            source_range: sample_source_range(),
            visibility: Some(Visibility::Private),
            signature: None,
            docstring: None,
        };
        let val: serde_json::Value = serde_json::to_value(&entry).unwrap();
        assert_eq!(val["name"], "obj");
        assert_eq!(val["visibility"], "Private");
        assert!(val["signature"].is_null());
        assert!(val["docstring"].is_null());
    }

    #[test]
    fn inspect_output_serde_roundtrip() {
        let output = InspectOutput {
            funcs: vec![FuncEntry {
                name: "fn1".into(),
                source_range: sample_source_range(),
                visibility: Some(Visibility::Public),
                signature: None,
                docstring: None,
                is_async: false,
            }],
            classes: vec![ClassEntry {
                name: "Cls".into(),
                source_range: sample_source_range(),
                visibility: None,
                signature: None,
                docstring: None,
            }],
            objects: vec![ObjectEntry {
                name: "obj".into(),
                source_range: sample_source_range(),
                visibility: None,
                signature: None,
                docstring: None,
            }],
        };
        let json = serde_json::to_string(&output).unwrap();
        let val: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(val["funcs"][0]["name"], "fn1");
        assert_eq!(val["classes"][0]["name"], "Cls");
        assert_eq!(val["objects"][0]["name"], "obj");
    }

    #[test]
    fn inspect_output_json_keys() {
        let output = InspectOutput {
            funcs: vec![],
            classes: vec![],
            objects: vec![],
        };
        let val: serde_json::Value = serde_json::to_value(&output).unwrap();
        let keys: std::collections::HashSet<&str> = val
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
