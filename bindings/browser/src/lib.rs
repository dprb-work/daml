#![forbid(unsafe_code)]

mod bridge;
mod diagnostic;

use daml::FormatOptions;
use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export type Tag = { name: string; binding?: string };
export type SemanticNode =
  | { kind: 'null'; tag?: Tag }
  | { kind: 'bool'; value: boolean; tag?: Tag }
  | { kind: 'number'; value: string; tag?: Tag }
  | { kind: 'string'; value: string; tag?: Tag }
  | { kind: 'list'; items: SemanticNode[]; tag?: Tag }
  | { kind: 'map'; entries: { key: string; value: SemanticNode }[]; tag?: Tag };
export type Origin =
  | { kind: 'input'; span: { start: number; end: number } }
  | { kind: 'default'; resource: string; pointer: string };
export type NodeOrigin = Origin & { tagOrigin?: Origin };
export type DamlError = Error & {
  name: 'DamlError'; category: 'parse' | 'native' | 'resolve' | 'semantic' | 'schema';
  kind: string; pointer?: string; path?: string; reference?: string;
  span?: { start: number; end: number };
  range?: { start: { line: number; character: number }; end: { line: number; character: number } };
  origin?: Origin; schema?: Origin;
};
"#;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(typescript_type = "SemanticNode")]
    pub type JsSemanticNode;
    #[wasm_bindgen(typescript_type = "Record<string, string>")]
    pub type JsSources;
    #[wasm_bindgen(typescript_type = "Record<string, NodeOrigin>")]
    pub type JsOrigins;
    #[wasm_bindgen(typescript_type = "number")]
    pub type JsWidth;
    #[wasm_bindgen(typescript_type = "string")]
    #[derive(Clone)]
    pub type JsSourceString;

    #[wasm_bindgen(js_namespace = globalThis, js_name = structuredClone, catch)]
    fn clone_snapshot(value: &JsValue) -> Result<JsValue, JsValue>;
}

fn options(width: Option<JsWidth>) -> Result<FormatOptions, JsValue> {
    let width = match width {
        None => 80.0,
        Some(value) => AsRef::<JsValue>::as_ref(&value).as_f64().ok_or_else(|| {
            diagnostic::error(
                "semantic",
                "InvalidLineWidth",
                "line width must be a number",
                Some(""),
            )
        })?,
    };
    if !width.is_finite() || width.fract() != 0.0 || !(1.0..=16_384.0).contains(&width) {
        return Err(diagnostic::error(
            "semantic",
            "InvalidLineWidth",
            "line width must be an integer from 1 through 16384",
            Some(""),
        ));
    }
    Ok(FormatOptions {
        line_width: width as usize,
    })
}

/// Parse bounded UTF-8-compatible source without completing or importing definitions.
#[wasm_bindgen]
pub fn parse(source: JsSourceString) -> Result<BrowserDocument, JsValue> {
    let value = bridge::string(
        source.as_ref(),
        bridge::MAX_BYTES,
        &bridge::DataPath::root(),
    )?;
    let document = daml::parse(value).map_err(|error| {
        let source = AsRef::<JsValue>::as_ref(&source)
            .as_string()
            .expect("admitted source string");
        diagnostic::parse(error, &source)
    })?;
    Ok(BrowserDocument { document, source })
}

/// Reflow native definition source through the same core formatter as native clients.
#[wasm_bindgen(js_name = formatDefinition)]
pub fn format_definition(
    source: JsSourceString,
    width: Option<JsWidth>,
) -> Result<String, JsValue> {
    let options = options(width)?;
    let text = bridge::string(
        source.as_ref(),
        bridge::MAX_BYTES,
        &bridge::DataPath::root(),
    )?;
    let definition = daml::parse_definition(text).map_err(|error| {
        let source = AsRef::<JsValue>::as_ref(&source)
            .as_string()
            .expect("admitted source string");
        diagnostic::parse(error, &source)
    })?;
    daml::format_definition(&definition, options)
        .map_err(|error| diagnostic::error("semantic", "FormatError", &error.to_string(), None))
}

/// Write an editable lossless value. Qualified assertions require Schemas.write.
#[wasm_bindgen]
pub fn write(value: JsSemanticNode, width: Option<JsWidth>) -> Result<String, JsValue> {
    let options = options(width)?;
    let value = bridge::decode(value.as_ref())?;
    daml::format_semantic(&value, options).map_err(diagnostic::semantic)
}

/// An immutable parsed source document. Returned value trees are owned editable copies.
#[wasm_bindgen(js_name = Document)]
pub struct BrowserDocument {
    document: daml::Document,
    source: JsSourceString,
}

#[wasm_bindgen(js_class = Document)]
impl BrowserDocument {
    #[wasm_bindgen(getter)]
    pub fn source(&self) -> JsSourceString {
        self.source.clone()
    }

    pub fn value(&self) -> Result<JsSemanticNode, JsValue> {
        Ok(bridge::document(&self.document)?.unchecked_into())
    }

    pub fn format(&self, width: Option<JsWidth>) -> Result<String, JsValue> {
        daml::format(&self.document, options(width)?)
            .map_err(|error| diagnostic::error("semantic", "FormatError", &error.to_string(), None))
    }
}

/// Explicit definition resources. No file discovery, network fetch, or Python is used.
#[wasm_bindgen(js_name = Bundle)]
pub struct BrowserBundle {
    bundle: daml::Bundle,
}

#[wasm_bindgen(js_class = Bundle)]
impl BrowserBundle {
    #[wasm_bindgen(constructor)]
    pub fn new(root: JsSourceString, sources: JsSources) -> Result<BrowserBundle, JsValue> {
        let root = bridge::resource_string(root.as_ref(), 4096, &bridge::DataPath::root())?;
        let sources = bridge::sources(sources.as_ref())?;
        let bundle = daml::Bundle::from_sources(root, sources).map_err(diagnostic::native)?;
        Ok(Self { bundle })
    }

    pub fn compile(
        &self,
        document: &BrowserDocument,
        expected: Option<JsSourceString>,
    ) -> Result<BrowserSchemas, JsValue> {
        let expected = expected
            .map(|value| bridge::string(value.as_ref(), 1024 * 1024, &bridge::DataPath::root()))
            .transpose()?;
        let schemas = self
            .bundle
            .compile(&document.document, expected.as_deref())
            .map_err(diagnostic::native)?;
        Ok(BrowserSchemas { schemas })
    }
}

/// A compiled immutable assertion context, including its source-visible tag bindings.
#[wasm_bindgen(js_name = Schemas)]
pub struct BrowserSchemas {
    schemas: daml::Schemas,
}

#[wasm_bindgen(js_class = Schemas)]
impl BrowserSchemas {
    pub fn resolve(
        &self,
        document: &BrowserDocument,
        expected: Option<JsSourceString>,
    ) -> Result<BrowserResolution, JsValue> {
        let expected = expected
            .map(|value| bridge::string(value.as_ref(), 1024 * 1024, &bridge::DataPath::root()))
            .transpose()?;
        let resolved = match expected.as_deref() {
            Some(expected) => self.schemas.resolve_as(&document.document, expected),
            None => self.schemas.resolve(&document.document),
        }
        .map_err(|error| diagnostic::resolve(error, document.document.source()))?;
        let (value, origins) = bridge::resolution(resolved.root())?;
        Ok(BrowserResolution {
            value,
            origins,
            source: document.source.clone(),
        })
    }

    pub fn write(&self, value: JsSemanticNode, width: Option<JsWidth>) -> Result<String, JsValue> {
        let options = options(width)?;
        let value = bridge::decode(value.as_ref())?;
        self.schemas
            .format_semantic(&value, options)
            .map_err(diagnostic::semantic)
    }
}

/// Owned completed data and provenance, independent of input/compiled handle lifetimes.
#[wasm_bindgen(js_name = Resolution)]
pub struct BrowserResolution {
    value: JsValue,
    origins: JsValue,
    source: JsSourceString,
}

#[wasm_bindgen(js_class = Resolution)]
impl BrowserResolution {
    #[wasm_bindgen(getter)]
    pub fn source(&self) -> JsSourceString {
        self.source.clone()
    }

    pub fn value(&self) -> Result<JsSemanticNode, JsValue> {
        Ok(clone_snapshot(&self.value)?.unchecked_into())
    }

    pub fn origins(&self) -> Result<JsOrigins, JsValue> {
        Ok(clone_snapshot(&self.origins)?.unchecked_into())
    }
}
