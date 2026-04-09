# Symbol Extraction Specification

## 1. Purpose

Define language-pack extraction contracts backed by Tree-sitter grammar queries.

## 2. Shared extraction rules

- Prefer grammar field-based extraction where available.
- Record both byte and line/column ranges.
- Continue extraction in presence of parser recovery nodes when safe.
- `Python` , `JS/TS` are prefered at the start of every iteration

## 3. Language packs

### Python

- Extract: functions, classes, imports, module-level assignments.
- Handle decorated definitions and async functions.

### JavaScript

- Extract: function declarations, function expressions, arrow functions, classes, methods, imports/exports.

### TypeScript / TSX

- Extract JS symbols plus interfaces, type aliases, enums.
- Use TSX grammar for JSX-bearing files.

### C

- Extract: function definitions/declarations, structs, enums, typedefs, includes.
- Distinguish declaration vs definition 'where possible'.

### C++

- Extract C symbols plus classes, namespaces, templates, aliases, method definitions.

### Rust

- Extract: functions, structs, enums, traits, impl blocks, use declarations, modules, const/static/type aliases.

### Go

- Extract: functions, methods with receiver, types, interfaces, imports, const/var declarations.

## 4. Output normalization

Each extracted symbol maps to canonical shape:

- `name`
- `kind`
- `language`
- `file`
- `source_range`
- optional: `signature`, `visibility`, `docstring`, `async`

## 5. Error tolerance policy

- Keep partial output if recoverable parse exists.
- Emit diagnostics for query compile failures or unsupported grammar drift.

## 6. Version policy

Query packs are tied to grammar versions in `Cargo.toml`. Any grammar upgrade requires:

1. Query validation pass.
2. Fixture/snapshot refresh.
3. Update to this spec.
