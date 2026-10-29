"""Derive in-memory DAML schema modules from Python configuration types.

Run from the repository root after building/installing DAML with its ``types``
extra; see docs/python.md for the setup command.
"""

from dataclasses import dataclass

import daml
from pydantic import BaseModel


class WorkerModel(BaseModel):
    name: str
    retries: int = 3


@dataclass
class WorkerRecord:
    name: str
    retries: int = 3


def show_resolution(label: str, module: daml.SchemaModule) -> None:
    schemas = daml.Schemas.compose([module])
    for source in (
        '!app.Worker { name: "demo" }',
        '!app.Worker { name: "demo"\nretries: 5 }',
        '!app.Worker { name: "demo"\nretries: "many" }',
    ):
        document = daml.parse(source)
        try:
            resolved = schemas.resolve(document, expected="app.Worker")
        except daml.ResolveError as error:
            print(f"{label}: {error.kind} at {error.path}: {error}")
        else:
            print(f"{label}: {resolved.to_python()}")


def main() -> None:
    pydantic_module = daml.schema_from_types({"app.Worker": WorkerModel})
    dataclass_module = daml.schema_from_types({"app.Worker": WorkerRecord})
    show_resolution("Pydantic", pydantic_module)
    show_resolution("dataclass", dataclass_module)


if __name__ == "__main__":
    main()
