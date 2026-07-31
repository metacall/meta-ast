# RFC 0011: MetaCall Client API Support (`metacall()`)

## Status

Accepted and Implemented.

## Implementation Notes

Implemented as designed with these notes:

- `metacall_handle` is excluded because argument layout varies per port (tag first in C/Node, handle first in Rust).
- Phase A and Phase B index all extracted symbols regardless of visibility flag.
- Rust `metacall_no_arg` and `metacall_untyped` map to `ClientCall`; `load::from_single_file` maps to `LoadFromFile`.
- Client-call reference edges flow as distinct inter-pod reference edges without altering load confidence.

## 1. Problem

The deploy scanner previously detected only `metacall_load_from_*` calls. Client function calls were not tracked:

```python
from metacall import metacall_load_from_file, metacall

metacall_load_from_file('node', ['auth/auth.js'])

def encrypt(text):
    return metacall('sign', text) # Untracked client call
```

Without client call tracking, Function Mesh topology missed function-level cross-language dependencies and call-site attribution.

## 2. API Surface

The scanner detects client invocation APIs across all supported ports:

| API | Ports | Target |
| --- | --- | --- |
| `metacall(name, ...)` | py, node, C, C++, Rust, Go | function name string |
| `metacall_await(name, ...)` | node, C, Go | function name string |
| `metacallfms(name, buffer)` | node | function name string |
| `metacallv(name, args[])`, `metacallt(...)` | C | function name string |
| `metacall_function(name)` | C | function name string |
| `metacall::metacall`, `metacall_no_arg`, `metacall_untyped` | Rust | function name string |
| `metacall.Call(...)`, `metacall.Await(...)` | Go | function name string |

## 3. Design

### 3.1 Model

`CallSite` includes `ClientCall` variant fields:

```rust
pub enum CallSiteVariant {
    LoadFromFile,
    LoadFromMemory,
    LoadFromPackage,
    LoadFromConfiguration,
    ClientCall,
}

pub struct CallSite {
    pub function_name: Option<String>,
    pub is_async: bool,
    // existing fields...
}
```

### 3.2 Two-Phase Function Resolution

`client_call::resolve_client_calls` resolves target functions:

1. **Phase A (Load-aware):** Matches call names against public symbols in files explicitly loaded by the calling file. Matches score `1.0` (unique) or `0.8` (ambiguous).
2. **Phase B (Global fallback):** Searches all project symbols if Phase A finds no match. Matches score `0.6` (unique) or `0.5` (ambiguous).
3. **Computed names:** Dynamic arguments cap edge confidence at `0.4`. Unresolved names produce a Warning diagnostic.

### 3.3 Graph Integration

Client-call edges are `EdgeKind::Reference` from calling file node to target symbol node. They participate in SCC analysis before pod partitioning, ensuring cross-language call cycles create proper cuts.

## 4. Impact

- `metacall.pods.json`: Includes `reference` edges for cross-language client calls.
- `metacall.mesh.json`: `cross_language_edges` attributes target deployment units and call-site files.
