# 0003-unresolved-import-policy

We defined a strict policy for handling unresolved imports to prevent silent failures and preserve dependency context.

Unresolved Relative Imports (specifiers starting with `.` or `/`) are treated as configuration errors and emit a warning `Diagnostic` containing the source range and path. Unresolved Non-Relative Imports are treated as third-party package dependencies and are mapped to a placeholder `External Node` in the directed graph rather than being silently discarded, preserving the complete structural architecture.

A resolver returns a miss when the target file does not exist; it never fabricates a path. A relative miss therefore warns, and a non-relative miss becomes an external node named by the bare specifier. Concretely: the JS family (JavaScript, TypeScript, TSX) probes existing files only through `resolve_js_family_import` and `probe_relative` with no fallback path, bare specifiers miss, extension probing appends so dotted basenames resolve, a Go package path needs its `.go` file, and a dots-only Python import needs its `__init__.py`.
