# 0002-scope-resolution-heuristics

We established two distinct modes for symbol reference resolution: Strict Scope Resolution and Heuristic Scope Resolution. 

By default, the scope builder limits lookups strictly to direct imports (distance = 1) or explicit re-exports. Under a `--heuristic-resolution` flag, transitive BFS lookup across the entire import graph is allowed, but resolves with a degraded confidence score (0.8 for transitive same-language, 0.6 for cross-language). This guarantees security tools high recall without corrupting the precision of standard language-compliant analysis.
