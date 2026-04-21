pub mod error;
pub mod extractor;
pub mod input;
pub mod language;
pub mod model;
pub mod output;
pub mod parser;

pub use error::{Diagnostic, Error, Severity};
pub use input::detect_language;
pub use language::{LangId, LanguageSpec};
pub use model::{Symbol, SymbolId, SymbolKind, Visibility};
