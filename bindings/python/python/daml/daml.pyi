from collections.abc import Iterable
from decimal import Decimal
from os import PathLike

_Value = None | bool | int | Decimal | str | list["_Value"] | dict[str, "_Value"]

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

class Document:
    """Parsed source and syntax-only tag assertions."""

    @property
    def source(self) -> str: ...
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
    def discover(document_path: str | PathLike[str]) -> Bundle | None:
        """Find the nearest bundle containing the document path."""
        ...

    def compile(self, document: Document, expected: str | None = None) -> Schemas:
        """Compile reachable definitions, optionally with an expected root."""
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

class ResolvedDocument:
    """Owned resolved snapshot with separately retained tag assertions."""

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
