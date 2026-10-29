"""Derive DAML temporal schemas and preserve precise values.

Run from the repository root after building DAML with its ``types`` extra;
see docs/python.md for installation details.
"""

from dataclasses import dataclass
from datetime import UTC, date, datetime, time

import daml
from pydantic import AwareDatetime, BaseModel, NaiveDatetime


class EventModel(BaseModel):
    day: date = date(2026, 10, 29)
    starts: time = time(9, 30, 0, 123456)
    deadline: AwareDatetime = datetime(2026, 10, 29, 10, 30, 0, 123456, tzinfo=UTC)
    local_deadline: NaiveDatetime = datetime(2026, 10, 29, 11)


@dataclass
class EventRecord:
    day: date = date(2026, 10, 29)
    starts: time = time(9, 30, 0, 123456)
    deadline: datetime = datetime(2026, 10, 29, 10, 30, 0, 123456, tzinfo=UTC)
    local_deadline: datetime = datetime(2026, 10, 29, 11)


def main() -> None:
    source = """!app.Event {
  day: !Date "2026-10-29"
  starts: !Time "09:30:00.123456789123"
  deadline: !DateTime "2026-10-29T10:30:00.123456789123Z"
  local_deadline: !DateTime "2026-10-29T11:00:00"
}"""

    for label, model in (("Pydantic", EventModel), ("dataclass", EventRecord)):
        module = daml.schema_from_types({"app.Event": model})
        schemas = daml.Schemas.compose([module])
        defaults = schemas.resolve(
            daml.parse("!app.Event"), expected="app.Event"
        ).to_python()
        print(
            f"{label} defaults: { {key: value.isoformat() for key, value in defaults.items()} }"
        )
        values = schemas.resolve(daml.parse(source), expected="app.Event").to_python()
        print(
            f"{label} supplied: { {key: value.isoformat() for key, value in values.items()} }"
        )
        try:
            values["starts"].to_time()
        except ValueError as error:
            print(f"  standard-library conversion rejected: {error}")
        else:
            raise RuntimeError("A sub-microsecond value was silently narrowed")


if __name__ == "__main__":
    main()
