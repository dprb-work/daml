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
  | { kind: 'default'; resource: string; pointer: string }
  | { kind: 'semantic' };
export type NodeOrigin = Origin & { tagOrigin?: Origin };
export type DamlError = Error & {
  name: 'DamlError'; category: 'parse' | 'native' | 'resolve' | 'semantic' | 'schema';
  kind: string; pointer?: string; path?: string; reference?: string;
  span?: { start: number; end: number };
  range?: { start: { line: number; character: number }; end: { line: number; character: number } };
  origin?: Origin; schema?: Origin;
};
export type SummaryValue =
  | { kind: 'null' }
  | { kind: 'bool'; value: boolean }
  | { kind: 'number'; value: string }
  | { kind: 'string'; value: string }
  | { kind: 'list' | 'map'; length: number };
export type NodeSummary = (
  | Exclude<SummaryValue, { kind: 'list' | 'map' }>
  | { kind: 'list'; length: number }
  | { kind: 'map'; length: number; fields: { key: string; summary: SummaryValue }[] }
) & { tag?: Tag };
export type StageEdit =
  | { op: 'replace'; path: string; value: SemanticNode }
  | { op: 'copy'; path: string; from: string }
  | { op: 'expect'; path: string; name: string };
export type CopyEdge = { path: string; from: string };
export type AdmissionStage = {
  defer(summary: NodeSummary, path: string): boolean;
  materialize(value: SemanticNode, copies: CopyEdge[], round: number):
    undefined | null | Uint8Array | StageEdit[] | Error;
  numberClass?(lexeme: string): number;
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
    #[wasm_bindgen(typescript_type = "string[]")]
    pub type JsAdditionalTypes;
    #[wasm_bindgen(typescript_type = "readonly string[]")]
    pub type JsJsonModules;
    #[wasm_bindgen(typescript_type = "AdmissionStage")]
    pub type JsAdmissionStage;
    #[wasm_bindgen(typescript_type = "StageEdit[]")]
    pub type JsStageEdits;
    #[wasm_bindgen(typescript_type = "Uint8Array | null")]
    pub type JsMeaning;

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
        additional_types: Option<JsAdditionalTypes>,
    ) -> Result<BrowserSchemas, JsValue> {
        let expected = expected
            .map(|value| bridge::string(value.as_ref(), 1024 * 1024, &bridge::DataPath::root()))
            .transpose()?;
        let additional_types =
            bridge::additional_types(additional_types.as_ref().map(AsRef::as_ref))?;
        let schemas = self
            .bundle
            .compile_with_types(
                &document.document,
                expected.as_deref(),
                additional_types.iter().map(String::as_str),
            )
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
    /// Composes explicit strict JSON module envelopes without fetching resources.
    #[wasm_bindgen(js_name = fromJsonModules)]
    pub fn from_json_modules(modules: JsJsonModules) -> Result<BrowserSchemas, JsValue> {
        let modules = bridge::json_modules(modules.as_ref())?;
        let schemas = daml::Schemas::compose(modules).map_err(diagnostic::schema)?;
        Ok(Self { schemas })
    }

    /// Checks supplied values and assertions without completing or changing data.
    /// Success does not guarantee that literal keys can be written as DAML source.
    #[wasm_bindgen(js_name = validateSemantic)]
    pub fn validate_semantic(
        &self,
        value: JsSemanticNode,
        expected: Option<JsSourceString>,
    ) -> Result<(), JsValue> {
        let mut budget = bridge::Budget::default();
        let expected = expected
            .map(|value| budget.text(value.as_ref(), &bridge::DataPath::root()))
            .transpose()?;
        let value = bridge::decode_with_budget(value.as_ref(), &mut budget)?;
        self.schemas
            .validate_semantic(&value, expected.as_deref())
            .map_err(|error| diagnostic::resolve(error, ""))
    }

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

    #[wasm_bindgen(js_name = resolveStaged)]
    pub fn resolve_staged(
        &self,
        document: &BrowserDocument,
        stage: JsAdmissionStage,
        expected: Option<JsSourceString>,
        initial_edits: Option<JsStageEdits>,
    ) -> Result<BrowserStagedResolution, JsValue> {
        let mut stage = bridge::Stage::new(stage.into())?;
        let expected = expected
            .map(|value| stage.budget.text(value.as_ref(), &bridge::DataPath::root()))
            .transpose()?;
        let initial_edits = initial_edits
            .map(|value| stage.edits(value.as_ref()))
            .transpose()?
            .unwrap_or_default();
        let resolved = self
            .schemas
            .resolve_staged(
                &document.document,
                expected.as_deref(),
                &initial_edits,
                &mut stage,
            )
            .map_err(|error| stage_error(error, document.document.source()))?;
        BrowserStagedResolution::snapshot(
            resolved.root(),
            resolved.meaning(),
            Some(document.source.clone()),
            &mut stage.budget,
        )
    }

    #[wasm_bindgen(js_name = resolveSemanticStaged)]
    pub fn resolve_semantic_staged(
        &self,
        value: JsSemanticNode,
        stage: JsAdmissionStage,
        expected: Option<JsSourceString>,
        initial_edits: Option<JsStageEdits>,
    ) -> Result<BrowserStagedResolution, JsValue> {
        let mut stage = bridge::Stage::new(stage.into())?;
        let value = bridge::decode_with_budget(value.as_ref(), &mut stage.budget)?;
        let expected = expected
            .map(|value| stage.budget.text(value.as_ref(), &bridge::DataPath::root()))
            .transpose()?;
        let initial_edits = initial_edits
            .map(|value| stage.edits(value.as_ref()))
            .transpose()?
            .unwrap_or_default();
        let resolved = self
            .schemas
            .resolve_semantic_staged(&value, expected.as_deref(), &initial_edits, &mut stage)
            .map_err(|error| stage_error(error, ""))?;
        BrowserStagedResolution::snapshot(
            resolved.root(),
            resolved.meaning(),
            None,
            &mut stage.budget,
        )
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

fn stage_error(error: daml::StageError<JsValue>, source: &str) -> JsValue {
    match error {
        daml::StageError::Resolve(error) => diagnostic::resolve(error, source),
        daml::StageError::Callback(error) | daml::StageError::Rejected(error) => error,
    }
}

/// Owned staged data. Semantic input has no authored source to expose.
#[wasm_bindgen(js_name = StagedResolution)]
pub struct BrowserStagedResolution {
    value: JsValue,
    origins: JsValue,
    source: Option<JsSourceString>,
    meaning: JsValue,
}

impl BrowserStagedResolution {
    fn snapshot(
        root: &daml::ResolvedNode<'_>,
        meaning: Option<&[u8]>,
        source: Option<JsSourceString>,
        budget: &mut bridge::Budget,
    ) -> Result<Self, JsValue> {
        let (value, origins) = bridge::resolution_with_budget(root, budget)?;
        let meaning = bridge::meaning(meaning, budget)?;
        Ok(Self {
            value,
            origins,
            source,
            meaning,
        })
    }
}

#[wasm_bindgen(js_class = StagedResolution)]
impl BrowserStagedResolution {
    #[wasm_bindgen(getter)]
    pub fn source(&self) -> Option<JsSourceString> {
        self.source.clone()
    }

    pub fn value(&self) -> Result<JsSemanticNode, JsValue> {
        Ok(clone_snapshot(&self.value)?.unchecked_into())
    }

    pub fn origins(&self) -> Result<JsOrigins, JsValue> {
        Ok(clone_snapshot(&self.origins)?.unchecked_into())
    }

    pub fn meaning(&self) -> Result<JsMeaning, JsValue> {
        Ok(clone_snapshot(&self.meaning)?.unchecked_into())
    }
}
