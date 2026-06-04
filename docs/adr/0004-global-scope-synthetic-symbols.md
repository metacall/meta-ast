# 0004-global-scope-synthetic-symbols

We introduced a synthetic `Module Body Symbol` (named `"<global>"` or `"<module>"`) for files containing top-level global execution blocks to represent module-level dependencies.

All references occurring outside any class, function, or method boundary are owned by this synthetic symbol node. This maintains a homogeneous `Symbol -> Symbol` reference edge schema in the directed graph, allows complete dependency tracing of global execution paths, and prevents dropping critical module-level script dependencies.
