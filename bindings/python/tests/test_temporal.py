"""Public Python contracts for intrinsic temporal values."""

import subprocess
import sys
from dataclasses import dataclass
from datetime import date, datetime, time, timedelta, timezone
from pathlib import Path

import daml
import pytest


def test_invalid_intrinsic_literals():
    for source in (
        '!Date "2023-02-29"',
        '!Date "2024-13-01"',
        '!Time "12:60"',
        '!Time "12:30:00.١"',
        '!Time "24:00:01"',
        '!Time "24:00:00.001"',
        '!DateTime "2026-10-29t09:00"',
        '!DateTime "2026-10-29T09:00-00:00"',
        '!DateTime "2026-10-29T09:00+23:60"',
    ):
        with pytest.raises(daml.ParseError) as caught:
            daml.parse(source)
        assert caught.value.kind == "InvalidTemporal"


def test_python_values_preserve_kind_precision_and_explicit_offset():
    date_value = daml.parse('!Date "0000-02-29"').to_python()
    time_value = daml.parse('!Time "T09:30:00,123456789123"').to_python()
    utc_value = daml.parse('!DateTime "2026-10-29T09Z"').to_python()
    local_value = daml.parse('!DateTime "2026-10-29T09"').to_python()

    assert isinstance(date_value, daml.Date)
    assert date_value.isoformat() == "0000-02-29"
    assert isinstance(time_value, daml.Time)
    assert time_value.isoformat() == "09:30:00.123456789123"
    assert isinstance(utc_value, daml.DateTime)
    assert utc_value.isoformat() == "2026-10-29T09:00:00Z"
    assert utc_value.offset_minutes == 0
    assert local_value.isoformat() == "2026-10-29T09:00:00"
    assert local_value.offset_minutes is None
    assert time_value.offset_minutes is None
    assert daml.Time("09:30Z").offset_minutes == 0
    with pytest.raises(ValueError):
        date_value.to_date()
    with pytest.raises(ValueError):
        time_value.to_time()
    leap = daml.DateTime("2016-12-31T23:59:60.5Z")
    with pytest.raises(ValueError):
        leap.to_datetime()


def test_exact_stdlib_conversions_and_immutable_values():
    date_value = daml.Date("2024-02-29")
    time_value = daml.Time("09:30:00.123456000")
    aware = daml.DateTime("2024-02-29T09:30:00+01:30")
    assert date_value.to_date() == date(2024, 2, 29)
    assert time_value.to_time() == time(9, 30, 0, 123456)
    assert aware.to_datetime() == datetime(
        2024, 2, 29, 9, 30, tzinfo=timezone(timedelta(hours=1, minutes=30))
    )
    with pytest.raises((AttributeError, TypeError)):
        date_value.value = "2024-01-01"
    for value in (
        daml.Date("0000-01-01"),
        daml.Time("23:59:60"),
        daml.Time("24:00"),
        daml.Time("09:30:00.123456789"),
    ):
        with pytest.raises(ValueError):
            value.to_date() if isinstance(value, daml.Date) else value.to_time()


def test_intrinsic_values_remain_distinct_from_strings():
    values = daml.parse('[!Date "2024-02-29"\n"2024-02-29"]').value()
    assert values["kind"] == "list"
    temporal, ordinary = values["items"]
    assert ordinary == {"kind": "string", "value": "2024-02-29"}
    written = daml.write(temporal)
    assert daml.parse(written).value() == temporal


def test_pydantic_and_dataclass_temporal_derivation_preserves_defaults():
    # Keep Pydantic optional for the package's base/custom-exporter surface.
    pytest.importorskip("pydantic")
    from pydantic import AwareDatetime, BaseModel, NaiveDatetime

    class Config(BaseModel):
        day: date = date(2024, 2, 29)
        clock: time = time(9, 30)
        deadline: AwareDatetime = datetime(2024, 2, 29, 9, 30, tzinfo=timezone.utc)
        local: NaiveDatetime = datetime(2024, 2, 29, 9, 30)

    @dataclass
    class Other:
        day: date = date(2020, 1, 2)

    module = daml.schema_from_types({"Config": Config, "Other": Other})
    schemas = daml.Schemas.compose([module])
    resolved = schemas.resolve(daml.parse("!Config"), expected="Config")
    resolved_value = resolved.to_python()
    dataclass_value = schemas.resolve(
        daml.parse("!Other"), expected="Other"
    ).to_python()
    assert isinstance(resolved_value["day"], daml.Date)
    assert resolved_value["day"].isoformat() == "2024-02-29"
    assert isinstance(resolved_value["clock"], daml.Time)
    assert resolved_value["deadline"].offset_minutes == 0
    assert resolved_value["local"].offset_minutes is None
    assert isinstance(dataclass_value["day"], daml.Date)
    assert dataclass_value["day"].isoformat() == "2020-01-02"
    for source in (
        '!Config { deadline: !DateTime "2024-02-29T09:30" }',
        '!Config { local: !DateTime "2024-02-29T09:30Z" }',
    ):
        with pytest.raises(daml.ResolveError):
            schemas.resolve(daml.parse(source), expected="Config")


def test_unrepresentable_temporal_constraints_are_not_silently_dropped():
    # Bounds use Pydantic metadata; base-only installations skip this projection.
    pytest.importorskip("pydantic")
    from pydantic import BaseModel, Field

    class Bounded(BaseModel):
        value: date = Field(default=date(2024, 1, 1), ge=date(2020, 1, 1))

    with pytest.raises((TypeError, ValueError), match="Bounded"):
        daml.schema_from_types({"Bounded": Bounded})


def test_base_import_and_custom_exporter_keep_temporals_without_site_packages():
    package = Path(daml.__file__).resolve().parent
    script = """import daml
class T: pass
module = daml.schema_from_types({'app.T': T}, exporter=lambda _: {'type': 'string'})
schemas = daml.Schemas.compose([module])
assert schemas.resolve(daml.parse('"ok"'), expected='app.T').to_python() == 'ok'
value = daml.parse('!Date "2024-02-29"').to_python()
assert isinstance(value, daml.Date)
assert value.isoformat() == "2024-02-29"
"""
    result = subprocess.run(
        [sys.executable, "-S", "-c", script],
        cwd=package.parent,
        env={"PYTHONPATH": str(package.parent)},
        capture_output=True,
        text=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
