# RFC 0013: Package-Level Import Targets

## Status

Proposed.

## 1. Problem

A Go import names a package, and a package is a directory. The graph model has
two target kinds: a file node for a path that was discovered, and an external
node for a string that could not be resolved. Neither represents a directory.

The Go resolver therefore answers a package import in one of two shapes.

1. Module root (`import "myproject"`). The resolver returns `None`
   (`src/language/go.rs:19-24`, the same rule in the caching resolver at
   `src/language/import_resolver.rs:292-297`). The builder treats a `None` for a
   non-relative specifier as an unresolved external dependency and names the
   node after the specifier (`src/graph/builder.rs:481-489`, name from
   `external_name` at `src/graph/builder.rs:578-586`). Measured result: an
   external node named `myproject`.
2. Subpackage (`import "myproject/internal/util"`). The resolver builds a file
   path from the module directory and the import remainder, adding the language
   extension (`src/language/go.rs:25-26`, mirrored in
   `src/language/import_resolver.rs:289-291`). If no discovered file has exactly
   that path, `add_import` (`src/graph/builder.rs:198-238`) creates an external
   node whose `raw_path` is that path. Measured result on a module with
   `internal/util/helper.go`: an external node named
   `/tmp/.../internal/util.go`, a file that does not exist, while the real
   member file is a file node with no import edge from the importer and the
   reference `util.Helper()` fails to resolve (`unresolved reference: 'util'`,
   `unresolved reference: 'Helper'`).

Four consequences follow.

- The file-level dependency graph misses the package. No import edge reaches
  the member files, so a cycle that runs through a package is invisible to the
  SCC pass and to the deployability hints.
- The dependency is not portable. The target name is an absolute path taken
  from the machine that ran the analysis. Two machines produce two different
  names for the same import.
- The shard index leaks that path. External nodes get the name
  `<language> external <escaped raw path>` (`src/output/shard/name.rs:56-64`),
  and external nodes belong to no file (`src/output/shard/name.rs:12-18`), so
  the path is written into the index as the identity of the dependency.
- Navigation cannot reach a package. The language server navigates on
  symbol-to-symbol reference edges (`src/graph/mod.rs:308-320`), and a package
  target holds no symbol.

Adjacent defect, tracked separately: the persisted import specifier keeps its
quotes (`import_specifier == "\"myproject\""`), which does not change the
analysis of this RFC.

## 2. Requirements and constraints

- R1, determinism. The same tree produces the same target identity on every
  machine and every run, and no absolute path reaches an output document.
- R2, no fabricated paths. The analysis must never name a file that does not
  exist.
- R3, compatibility. Shard readers refuse a schema version they do not know
  (`src/output/shard/file.rs:24`, `SHARD_SCHEMA_VERSION = 4`; reader rules at
  `src/output/shard/mod.rs:235` and `:414`), so an identity change needs a
  version bump and a defined story for indexes written by older builds.
- R4, navigation. An editor must reach the package and its members from the
  import site.
- R5, deploy semantics. Pods partition by language plus Import and Reference
  edges. A package target must not add an edge kind or inflate a confidence.
- R6, no new dependencies, and every produced order stays deterministic.

## 3. Options

| Option | Graph shape | Determinism | Shard impact | Navigation | Deploy impact |
| --- | --- | --- | --- | --- | --- |
| A. Package node | New node kind; `File -> Package` import edges; `Package -> File` membership edges | High: identity is the project-relative directory | New name shape plus a version bump | Import site reaches the package, members follow membership edges | Membership edges must collapse before partitioning, or pods gain a package member |
| B. No target | `None` for every package import, consumers group by directory | High | None | Unchanged (no target to navigate to) | No edge, so no package cycle is visible |
| C. Representative file | Keep a file target, but point it at a real member file chosen by rule | Low: depends on directory contents and rule | None | Import site reaches one arbitrary file | The package's other members stay unreachable |
| D. Package node plus file edges to every member | As A, without a membership edge kind: `File -> File` for every member | High | Many more edges | Import site reaches each member directly | Duplicates the package identity per member and inflates the import degree of the importer |

Option B rejects the dependency information that the resolver already has.
Option C needs a rule that agrees with the language (which file declares the
imported identifier?), depends on the file system at resolution time, and is
undefined for an empty package or a member outside the extraction set. Option D
turns one dependency into N edges and makes the same package appear as N
targets in any aggregation.

## 4. Decision and rationale

Take option A, with these rules.

- Identity: the project-root-relative directory of the package, kept as a
  portable path. The language comes from the importing file's language.
- Edges: `File -> Package` for the import, `Package -> File` for each
  extracted member file. Membership edges carry full confidence and belong to
  the same `Import` kind as ordinary imports; no new edge kind is introduced.
- An empty package (no extracted member) is still a node with no membership
  edges. Nothing is fabricated.
- A package import never falls back to an external node named after a
  specifier. Only a workspace without a module file keeps the external
  fallback, and that fallback stays a string, never a path.

Rationale: the package is the unit the source language names, so it is the
right identity; membership edges give the SCC pass and the deployability hints
the real reachability without touching confidence rules; and a single new node
kind gives the shard layer one nameable target instead of an absolute path.

## 5. Impact

- Graph model. One additional node kind. The enum is `#[non_exhaustive]`
  (`src/graph/node.rs:17-22`), so consumers that match on it compile unchanged
  and must handle the new arm when they choose to. `graph-model.md` gains the
  node and both edge directions.
- Shard format. External package targets become package names of the shape
  `<language> package <escaped portable directory>`. `SHARD_SCHEMA_VERSION`
  moves from 4 to 5, and the reader keeps refusing unknown versions with the
  expected version reported.
- Navigation. The import site reaches the package node, and the package node
  reaches its members, so a symbol lookup can continue on file scope. Symbol to
  symbol navigation is unchanged.
- Deploy. The partition step must treat a package node as its member files
  (collapse membership first) so pods keep file granularity.
- Diagnostics. `unresolved reference` diagnostics for package members
  disappear only once membership edges exist; the resolver change alone does
  not fix the reference pass.

## 6. Migration and compatibility

Two steps, in this order.

1. Stop fabricating paths. A subpackage import becomes `None` (or an external
   node named after the specifier) exactly as the module root case does today.
   No model change, no schema change. This step is safe to ship alone and it
   removes the absolute path from every index.
2. Add the package node and the membership edges, bump the shard schema, and
   define the reader rule for version 4 indexes (either refuse them, as the
   reader does today, or map the old external target to the new package node
   when the name is a directory-shaped path).

Step 1 changes observable behaviour, so the interim contract test must be
updated in the same change, never deleted. Step 2 is a model change and needs
its own review.

## 7. Test plan

- Resolver units: module root returns `None`; subpackage returns the package
  identity; a workspace without a module file keeps the specifier fallback; a
  relative import stays rejected.
- Graph: the import edge ends at a package node, membership edges reach each
  member file, and no edge target is a path without a file.
- Reference resolution: after membership edges exist, `util.Helper()` resolves
  or reports a precise diagnostic.
- Shard round trip: export and restore preserve the package node and its
  edges; a version 4 index is refused with the expected version reported; two
  exports are byte identical; no shard name contains an absolute path.
- Deploy: a Go-only project keeps one pod for the module, and the package node
  does not appear as a pod of its own.

## 8. Open questions

- Should the package node carry the package clause name from the source
  (`package util`) in addition to the directory? Files in one directory may
  disagree with the directory name.
- Should other languages with directory-shaped imports share the node kind?
  Python resolves a directory import to `__init__.py`, which exists, so the
  immediate need is Go only.
- Does the deploy pipeline want package granularity in pods, or should it
  always collapse membership to file granularity?
- Multi-module workspaces: `find_go_module` walks parents from the project root
  and returns the first `go.mod` (`src/language/import_resolver.rs:136-160`), so
  a `go.work` workspace or a nested module resolves against the wrong module.
  Which module owns a package path?
