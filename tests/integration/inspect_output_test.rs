use std::path::Path;

fn flatten_symbols(result: &meta_ast::extractor::ExtractionResult) -> Vec<meta_ast::model::Symbol> {
    result
        .files
        .iter()
        .flat_map(|f| f.symbols.iter().cloned())
        .collect()
}

fn extract_python_symbols(root: &Path) -> Vec<meta_ast::model::Symbol> {
    let files = meta_ast::input::discover_files(root, None).unwrap();
    let result = meta_ast::extractor::extract(&files);
    flatten_symbols(&result)
}

fn serialize_to_json(symbols: &[meta_ast::model::Symbol]) -> serde_json::Value {
    let json_str = meta_ast::output::inspect::serialize_inspect(
        symbols,
        &meta_ast::output::OutputFormat::Json,
    )
    .unwrap();
    serde_json::from_str(&json_str).unwrap()
}

#[test]
fn inspect_output_has_required_top_level_keys() {
    let symbols = extract_python_symbols(Path::new("tests/fixtures/python"));
    let parsed = serialize_to_json(&symbols);

    let obj = parsed.as_object().expect("root should be object");
    assert!(obj.contains_key("funcs"), "must have funcs key");
    assert!(obj.contains_key("classes"), "must have classes key");
    assert!(obj.contains_key("objects"), "must have objects key");
    assert_eq!(
        obj.len(),
        3,
        "should have exactly 3 top-level keys, got {}: {:?}",
        obj.len(),
        obj.keys().collect::<Vec<_>>()
    );
}

#[test]
fn func_entry_has_required_fields() {
    let symbols = extract_python_symbols(Path::new("tests/fixtures/python"));
    let parsed = serialize_to_json(&symbols);

    let funcs = parsed["funcs"].as_array().expect("funcs should be array");
    if funcs.is_empty() {
        return;
    }
    for func in funcs {
        let obj = func
            .as_object()
            .unwrap_or_else(|| panic!("func entry should be object, got {func}"));

        assert!(obj.contains_key("name"), "func missing name: {obj:?}");
        assert!(
            obj.contains_key("source_range"),
            "func missing source_range: {obj:?}"
        );
        assert!(obj.contains_key("async"), "func missing async: {obj:?}");

        let sr = &func["source_range"];
        let sr_obj = sr
            .as_object()
            .unwrap_or_else(|| panic!("func source_range should be object, got {sr}"));
        assert!(
            sr_obj.contains_key("byte_start"),
            "source_range missing byte_start"
        );
        assert!(
            sr_obj.contains_key("byte_end"),
            "source_range missing byte_end"
        );
        assert!(sr_obj.contains_key("start"), "source_range missing start");
        assert!(sr_obj.contains_key("end"), "source_range missing end");

        let start = &sr["start"];
        let start_obj = start
            .as_object()
            .unwrap_or_else(|| panic!("source_range.start should be object, got {start}"));
        assert!(start_obj.contains_key("line"), "start missing line");
        assert!(start_obj.contains_key("column"), "start missing column");

        let end = &sr["end"];
        let end_obj = end
            .as_object()
            .unwrap_or_else(|| panic!("source_range.end should be object, got {end}"));
        assert!(end_obj.contains_key("line"), "end missing line");
        assert!(end_obj.contains_key("column"), "end missing column");
    }
}

#[test]
fn class_entry_has_required_fields() {
    let symbols = extract_python_symbols(Path::new("tests/fixtures/python"));
    let parsed = serialize_to_json(&symbols);

    let classes = parsed["classes"]
        .as_array()
        .expect("classes should be array");
    if classes.is_empty() {
        return;
    }
    for class in classes {
        let obj = class
            .as_object()
            .unwrap_or_else(|| panic!("class entry should be object, got {class}"));

        assert!(obj.contains_key("name"), "class missing name: {obj:?}");
        assert!(
            obj.contains_key("source_range"),
            "class missing source_range: {obj:?}"
        );

        let sr = &class["source_range"];
        let sr_obj = sr
            .as_object()
            .unwrap_or_else(|| panic!("class source_range should be object, got {sr}"));
        assert!(
            sr_obj.contains_key("byte_start"),
            "source_range missing byte_start"
        );
        assert!(
            sr_obj.contains_key("byte_end"),
            "source_range missing byte_end"
        );
        assert!(sr_obj.contains_key("start"), "source_range missing start");
        assert!(sr_obj.contains_key("end"), "source_range missing end");
    }
}

#[test]
fn object_entry_has_required_fields() {
    let symbols = extract_python_symbols(Path::new("tests/fixtures/python"));
    let parsed = serialize_to_json(&symbols);

    let objects = parsed["objects"]
        .as_array()
        .expect("objects should be array");
    if objects.is_empty() {
        return;
    }
    for obj_val in objects {
        let obj = obj_val
            .as_object()
            .unwrap_or_else(|| panic!("object entry should be object, got {obj_val}"));

        assert!(obj.contains_key("name"), "object missing name: {obj:?}");
        assert!(
            obj.contains_key("source_range"),
            "object missing source_range: {obj:?}"
        );

        let sr = &obj_val["source_range"];
        let sr_obj = sr
            .as_object()
            .unwrap_or_else(|| panic!("object source_range should be object, got {sr}"));
        assert!(
            sr_obj.contains_key("byte_start"),
            "source_range missing byte_start"
        );
        assert!(
            sr_obj.contains_key("byte_end"),
            "source_range missing byte_end"
        );
        assert!(sr_obj.contains_key("start"), "source_range missing start");
        assert!(sr_obj.contains_key("end"), "source_range missing end");
    }
}

#[test]
fn inspect_output_types_are_correct() {
    let symbols = extract_python_symbols(Path::new("tests/fixtures/python"));
    let parsed = serialize_to_json(&symbols);

    for func in parsed["funcs"].as_array().unwrap_or(&vec![]) {
        assert!(func["name"].is_string(), "func.name should be string");
        assert!(
            func["source_range"].is_object(),
            "func.source_range should be object"
        );
        assert!(func["async"].is_boolean(), "func.async should be boolean");
        if let Some(v) = func.get("signature") {
            assert!(
                v.is_string() || v.is_null(),
                "func.signature should be string or null, got {v}"
            );
        }
        if let Some(v) = func.get("visibility") {
            assert!(
                v.is_string() || v.is_null(),
                "func.visibility should be string or null, got {v}"
            );
        }
        if let Some(v) = func.get("docstring") {
            assert!(
                v.is_string() || v.is_null(),
                "func.docstring should be string or null, got {v}"
            );
        }
        let sr = &func["source_range"];
        assert!(
            sr["byte_start"].is_number(),
            "source_range.byte_start should be number"
        );
        assert!(
            sr["byte_end"].is_number(),
            "source_range.byte_end should be number"
        );
        assert!(
            sr["start"].is_object(),
            "source_range.start should be object"
        );
        assert!(sr["end"].is_object(), "source_range.end should be object");
        assert!(
            sr["start"]["line"].is_number(),
            "start.line should be number"
        );
        assert!(
            sr["start"]["column"].is_number(),
            "start.column should be number"
        );
        assert!(sr["end"]["line"].is_number(), "end.line should be number");
        assert!(
            sr["end"]["column"].is_number(),
            "end.column should be number"
        );
    }

    for class in parsed["classes"].as_array().unwrap_or(&vec![]) {
        assert!(class["name"].is_string(), "class.name should be string");
        assert!(
            class["source_range"].is_object(),
            "class.source_range should be object"
        );
        if let Some(v) = class.get("signature") {
            assert!(
                v.is_string() || v.is_null(),
                "class.signature should be string or null, got {v}"
            );
        }
        if let Some(v) = class.get("visibility") {
            assert!(
                v.is_string() || v.is_null(),
                "class.visibility should be string or null, got {v}"
            );
        }
        if let Some(v) = class.get("docstring") {
            assert!(
                v.is_string() || v.is_null(),
                "class.docstring should be string or null, got {v}"
            );
        }
    }

    for obj_val in parsed["objects"].as_array().unwrap_or(&vec![]) {
        assert!(obj_val["name"].is_string(), "object.name should be string");
        assert!(
            obj_val["source_range"].is_object(),
            "object.source_range should be object"
        );
    }
}

#[test]
fn empty_inspect_output_is_valid() {
    let symbols: Vec<meta_ast::model::Symbol> = vec![];
    let parsed = serialize_to_json(&symbols);

    let obj = parsed.as_object().expect("root should be object");
    assert!(obj.contains_key("funcs"), "must have funcs key");
    assert!(obj.contains_key("classes"), "must have classes key");
    assert!(obj.contains_key("objects"), "must have objects key");
    assert_eq!(obj.len(), 3, "should have exactly 3 top-level keys");

    let funcs = parsed["funcs"].as_array().expect("funcs should be array");
    let classes = parsed["classes"]
        .as_array()
        .expect("classes should be array");
    let objects = parsed["objects"]
        .as_array()
        .expect("objects should be array");

    assert!(funcs.is_empty(), "funcs should be empty for empty input");
    assert!(
        classes.is_empty(),
        "classes should be empty for empty input"
    );
    assert!(
        objects.is_empty(),
        "objects should be empty for empty input"
    );
}
