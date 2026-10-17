//! DAML — Definitively Asserted Markup Language.
//!
//! Explicit collections, unquoted keys, source-backed values, and deeply nested
//! tags, without YAML's implicit coercions or indentation-based ownership.
//! Parsing preserves assertions; schema resolution and construction are separate.
//!
//! ```
//! let document = daml::parse(r#"!app.Config {
//!   adapter: !network.Adapter
//!   name: "demo"
//! }"#)?;
//! let name = document.get(document.root(), "name").unwrap();
//! assert_eq!(document.string(name), Some("demo"));
//! assert_eq!(document.tag(document.root()), Some("app.Config"));
//! # Ok::<(), daml::ParseError>(())
//! ```
//!
//! Complete a partial value from schema defaults, then enforce all constraints:
//!
//! ```
//! let module = daml::SchemaModule::from_daml(r#"{
//!   types: {
//!     app.Config: {
//!       schema: {
//!         type: "object"
//!         properties: {
//!           timeout: {
//!             type: "integer"
//!             minimum: 1
//!             default: 30
//!           }
//!         }
//!         required: ["timeout"]
//!         additionalProperties: false
//!       }
//!     }
//!   }
//! }"#)?;
//! let schemas = daml::Schemas::compose([module])?;
//! let supplied = daml::parse("!app.Config")?;
//! let resolved = schemas.resolve(&supplied)?;
//! assert_eq!(resolved.root().get("timeout").unwrap().to_json(), 30);
//! assert!(supplied.get(supplied.root(), "timeout").is_none());
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::borrow::Cow;

mod json_backend;
mod model;
mod parser;
mod pointer;
mod resolve;
mod resolved;
mod schema;
mod schema_error;
mod strict_json;
mod strings;
mod tag_keywords;

pub use model::{Document, Entry, ErrorKind, Limits, Node, ParseError, Span, StringValue, Value};
pub use resolved::{
    ErrorOrigin, Origin, ResolutionLimits, ResolveError, ResolveErrorKind, ResolvedDocument,
    ResolvedNode, ResolvedValue, SchemaLocation,
};
pub use schema::{SchemaModule, Schemas};
pub use schema_error::{SchemaError, SchemaErrorKind};

/// Parses one complete DAML value with default resource bounds.
///
/// Passing a `String` transfers its ownership; passing `&str` copies the source
/// once. Unchanged string contents, keys, tags, and numbers retain source spans.
/// The document owns its source so it may outlive the caller's input variable.
///
/// # Errors
///
/// Returns a structured syntax or resource error. It never resolves tag names,
/// invokes application constructors, or accepts trailing nontrivia content.
pub fn parse<'a>(source: impl Into<Cow<'a, str>>) -> Result<Document, ParseError> {
    parse_with_limits(source, Limits::default())
}

/// Parses one complete value with explicit resource bounds.
///
/// # Errors
///
/// Returns syntax/resource errors as [`parse`] does. A requested nesting limit
/// above 128 is rejected rather than allowing unbounded recursive parsing.
pub fn parse_with_limits<'a>(
    source: impl Into<Cow<'a, str>>,
    limits: Limits,
) -> Result<Document, ParseError> {
    let source = source.into();
    parser::check_input(source.as_ref(), limits)?;
    let source = source.into_owned();
    let root = parser::parse(&source, limits)?;
    Ok(Document { source, root })
}
