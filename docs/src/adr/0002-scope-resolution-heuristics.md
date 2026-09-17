# 0002-scope-resolution-heuristics

We resolve symbol references across files using a single, unified scope resolution strategy: transitive BFS lookup over the import graph.

Rather than maintaining separate "strict" and "heuristic" CLI flags, we build a flattened scope cache for each file. The resolver crawls the import graph using a breadth-first search (BFS). Confidence scores decay based on import distance and language boundaries:
- **1.0**: Reference is local (distance = 0) or direct, same-language (distance = 1).
- **0.8**: Reference is transitive, same-language (distance > 1).
- **0.6**: Reference is cross-language (distance >= 1).

This keeps the CLI interface clean and guarantees security or dependency tools high recall by default, while still capturing precision through the decayed confidence scores.

Direct imports expose names through their bindings. A star import exposes every public name; a named import exposes its original under its local spelling, including when both spellings are equal. A direct pair whose records are all precise exposes exactly its declared names; any unfiltered record keeps file scope while each rename still adds its local spelling. Deeper levels stay file-level: bindings describe direct imports, and re-export analysis is out of scope.
