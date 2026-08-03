# 0003-unresolved-import-policy

We defined a strict policy for handling unresolved imports to prevent silent failures and preserve dependency context.

Unresolved Relative Imports (specifiers starting with `.` or `/`) are treated as configuration errors and emit a warning `Diagnostic` containing the source range and path. Unresolved Non-Relative Imports are treated as third-party package dependencies and are mapped to a placeholder `External Node` in the directed graph rather than being silently discarded, preserving the complete structural architecture.
