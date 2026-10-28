from collections.abc import Iterable
from decimal import Decimal
from os import PathLike
from typing import Literal, NotRequired, Protocol, TypedDict

_Value = None | bool | int | Decimal | str | list["_Value"] | dict[str, "_Value"]

class _SemanticTag(TypedDict):
    name: str
    binding: NotRequired[str]

class _SemanticNull(TypedDict):
    kind: Literal["null"]
    tag: NotRequired[_SemanticTag]

class _SemanticBool(TypedDict):
    kind: Literal["bool"]
    value: bool
    tag: NotRequired[_SemanticTag]

class _SemanticNumber(TypedDict):
    kind: Literal["number"]
    value: str
    tag: NotRequired[_SemanticTag]

class _SemanticString(TypedDict):
    kind: Literal["string"]
    value: str
    tag: NotRequired[_SemanticTag]

class _SemanticList(TypedDict):
    kind: Literal["list"]
    items: list[_SemanticNode]
    tag: NotRequired[_SemanticTag]

class _SemanticEntry(TypedDict):
    key: str
    value: _SemanticNode

class _SemanticMap(TypedDict):
    kind: Literal["map"]
    entries: list[_SemanticEntry]
    tag: NotRequired[_SemanticTag]

_SemanticNode = (
    _SemanticNull
    | _SemanticBool
    | _SemanticNumber
    | _SemanticString
    | _SemanticList
    | _SemanticMap
)

class _SummaryList(TypedDict):
    kind: Literal["list"]
    length: int
    tag: NotRequired[_SemanticTag]

class _SummaryMapValue(TypedDict):
    kind: Literal["map"]
    length: int

_SummaryValue = (
    _SemanticNull
    | _SemanticBool
    | _SemanticNumber
    | _SemanticString
    | _SummaryList
    | _SummaryMapValue
)

class _SummaryField(TypedDict):
    key: str
    summary: _SummaryValue

class _SummaryMap(_SummaryMapValue):
    fields: list[_SummaryField]
    tag: NotRequired[_SemanticTag]

_NodeSummary = (
    _SemanticNull
    | _SemanticBool
    | _SemanticNumber
    | _SemanticString
    | _SummaryList
    | _SummaryMap
)

class _ReplaceEdit(TypedDict):
    op: Literal["replace"]
    path: str
    value: _SemanticNode

# "from" is a wire key, not a Python identifier.
_CopyEdit = TypedDict("_CopyEdit", {"op": Literal["copy"], "path": str, "from": str})

class _ExpectEdit(TypedDict):
    op: Literal["expect"]
    path: str
    name: str

_StageEdit = _ReplaceEdit | _CopyEdit | _ExpectEdit
_InitialEdit = _ReplaceEdit | _ExpectEdit
_CopyEdge = TypedDict("_CopyEdge", {"path": str, "from": str})

class _AdmissionStage(Protocol):
    """Synchronous callbacks; optional number_class(lexeme: str) -> int refines
    native equality with a non-bool u32. Summaries expose only own tag metadata;
    immediate fields omit tags and descendants. Raised exceptions are fatal.
    """

    def defer(self, summary: _NodeSummary, path: str) -> bool: ...
    def materialize(
        self, value: _SemanticNode, copies: list[_CopyEdge], round: int
    ) -> None | bytes | list[_StageEdit] | BaseException:
        """Return Ready, bounded edits, or a candidate-local rejection.

        Value and copies are fresh exports, not mutable candidate handles.
        Round resets to zero for each candidate replay; edits advance it.
        """
        ...

class ParseError(ValueError):
    kind: str
    span: tuple[int, int]
    path: None

class NativeError(ValueError):
    kind: str
    span: tuple[int, int] | None
    path: str | None
    reference: str | None

class SchemaError(ValueError):
    kind: str
    schema: tuple[str, str] | None

class ResolveError(ValueError):
    kind: str
    path: str
    span: tuple[int, int] | None
    schema: tuple[str, str] | None
    origin: tuple[str, str] | None

class SemanticError(ValueError):
    kind: str
    pointer: str

class Document:
    """Parsed source and syntax-only tag assertions."""

    @property
    def source(self) -> str: ...
    def value(self) -> _SemanticNode:
        """Return a fresh editable semantic tree with exact number strings."""
        ...

    def to_python(self) -> _Value:
        """Return an owned value tree, dropping tag metadata."""
        ...

    def tags(self) -> dict[str, str]:
        """Return asserted tags keyed by RFC 6901 pointer."""
        ...

    def format(self, line_width: int = 80) -> str:
        """Format this syntax tree without resolving it."""
        ...

class SchemaModule:
    """Validated standalone schema input for explicit composition."""

    @staticmethod
    def from_json(source: str) -> SchemaModule:
        """Load a schema module from strict JSON."""
        ...

    @staticmethod
    def from_daml(source: str) -> SchemaModule:
        """Load a schema module from DAML syntax."""
        ...

class Bundle:
    """An explicitly selected native bundle."""

    @staticmethod
    def open(root: str | PathLike[str]) -> Bundle:
        """Open the bundle rooted at this directory."""
        ...

    @staticmethod
    def from_sources(root: str, sources: dict[str, str]) -> Bundle:
        """Own an explicit UTF-8 resource table without filesystem fallback."""
        ...

    @staticmethod
    def discover(document_path: str | PathLike[str]) -> Bundle | None:
        """Find the nearest bundle containing the document path."""
        ...

    def compile(
        self,
        document: Document,
        expected: str | None = None,
        additional_types: Iterable[str] | None = None,
    ) -> Schemas:
        """Load reachable and explicit types without making root policy sticky."""
        ...

class Schemas:
    """Immutable composed schema registry."""

    @staticmethod
    def compose(modules: Iterable[SchemaModule]) -> Schemas:
        """Compose modules without overriding duplicate names."""
        ...

    def resolve(
        self, document: Document, expected: str | None = None
    ) -> ResolvedDocument:
        """Complete defaults and validate assertions and optional root."""
        ...

    def resolve_semantic(
        self, value: _SemanticNode, expected: str | None = None
    ) -> ResolvedDocument:
        """Complete and validate an editable semantic tree, retaining bindings."""
        ...

    def resolve_staged(
        self,
        document: Document,
        stage: _AdmissionStage,
        expected: str | None = None,
        initial_edits: list[_InitialEdit] | tuple[_InitialEdit, ...] | None = None,
    ) -> ResolvedDocument:
        """Complete through the shared native stage, then strictly validate.

        None means no initial edits; explicit lists and tuples are admitted strictly.
        """
        ...

    def resolve_semantic_staged(
        self,
        value: _SemanticNode,
        stage: _AdmissionStage,
        expected: str | None = None,
        initial_edits: list[_InitialEdit] | tuple[_InitialEdit, ...] | None = None,
    ) -> ResolvedDocument:
        """Admit exact semantic input and stage without source roundtripping.

        None means no initial edits; explicit lists and tuples are admitted strictly.
        """
        ...

    def write(self, value: _SemanticNode, line_width: int = 80) -> str:
        """Write source only when visible names preserve every tag binding."""
        ...

class ResolvedDocument:
    """Owned semantic snapshot retaining exact numbers and tag bindings."""

    def meaning(self) -> bytes | None:
        """Return the selected owned Ready key, or None for plain resolution."""
        ...

    def value(self) -> _SemanticNode:
        """Return a fresh editable semantic tree with opaque binding IDs."""
        ...

    def to_python(self) -> _Value:
        """Return an owned copy of the resolved snapshot."""
        ...

    def tags(self) -> dict[str, str]:
        """Return asserted and generated tags keyed by RFC 6901 pointer."""
        ...

def parse(source: str) -> Document:
    """Parse DAML syntax while retaining source and tag assertions."""
    ...

def format_definition(source: str, line_width: int = 80) -> str:
    """Parse and format a native type definition."""
    ...

def write(value: _SemanticNode, line_width: int = 80) -> str:
    """Write unqualified semantic data as canonical DAML source."""
    ...
