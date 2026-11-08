"""Complete native cron defaults, inspect their types, and edit a pattern."""

from pathlib import Path

import daml


def main() -> None:
    root = Path(__file__).parent / "cron"
    document = daml.parse((root / "data.daml").read_text(encoding="utf-8"))
    schemas = daml.Bundle.open(root).compile(document)
    resolved = schemas.resolve(document)
    values = resolved.to_python()
    for name, value in values.items():
        if not isinstance(value, daml.Cron):
            raise TypeError(f"{name} lost its intrinsic Cron kind")
        print(f"{name}: {value.expression}")

    editable = resolved.value()
    for entry in editable["entries"]:
        if entry["key"] == "month_end":
            entry["value"] = {"kind": "cron", "value": "0 0 18 L * *"}
            break
    else:
        raise ValueError("resolved schedule has no month_end field")
    written = schemas.write(editable)
    edited = schemas.resolve(daml.parse(written)).to_python()
    if edited["month_end"] != daml.Cron("0 0 18 L * *"):
        raise ValueError("edited cron pattern did not survive contextual writing")
    print(f"edited month_end: {edited['month_end']}")

    try:
        daml.Cron("0 9 L * *")
    except ValueError as error:
        print(f"five-field pattern rejected: {error}")
    else:
        raise ValueError("a five-field pattern was incorrectly accepted")


if __name__ == "__main__":
    main()
