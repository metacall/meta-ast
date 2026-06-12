# 0001-stateful-import-resolver

We introduced a stateful `ImportResolver` trait utilizing thread-safe interior mutability (`RwLock` and `OnceLock`) for caching module boundaries and file existence checks, replacing the previous stateless function pointer resolution. 

This design satisfies Rust's thread-safety constraints (`Send + Sync`) to allow gradual migration of 8 languages, prepares the pipeline for parallelized import resolution under Rayon, and supports persistent cache reuse across incremental compilation runs.
