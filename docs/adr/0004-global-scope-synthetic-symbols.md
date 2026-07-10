# 0004-global-scope-synthetic-symbols

We proposed introducing a synthetic `Module Body Symbol` (named `"<global>"` or `"<module>"`) for files containing top-level global execution blocks to represent module-level dependencies. Under that proposed schema, references occurring outside any class, function, or method boundary would be owned by this synthetic symbol.

**Current MVP Status**:
We do not yet generate this synthetic symbol. In the current implementation, references that occur at the top-level (outside of functions or classes) have a `None` source symbol and are skipped during scope resolution. 

Implementing the synthetic module body symbol is deferred to a future phase to maintain simplicity in the initial graph schema.
