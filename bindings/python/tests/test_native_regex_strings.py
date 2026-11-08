"""Native Regex stays raw across Python's real bundle compile/resolve boundary."""

import json

import daml
import pytest


@pytest.mark.parametrize("delimiter", ['"""', "'''"])
@pytest.mark.parametrize("eol", ["\n", "\r\n", "\r"])
@pytest.mark.parametrize("on_disk", [False, True])
def test_native_multiline_regex_preserves_backslashes(
    tmp_path, delimiter, eol, on_disk
):
    source = (
        f"!String {{ pattern: !Regex {delimiter}{eol}"
        rf"  \d+\\x{eol}"
        f"  {delimiter}{eol}}}"
    )
    sources = {"daml.bundle": "{}", "Pattern.daml": source}
    if on_disk:
        for name, contents in sources.items():
            (tmp_path / name).write_bytes(contents.encode("utf-8"))
        bundle = daml.Bundle.open(tmp_path)
    else:
        bundle = daml.Bundle.from_sources(".", sources)
    valid = daml.parse(json.dumps("١٢\\x", ensure_ascii=False))
    schemas = bundle.compile(valid, expected="self.Pattern")
    assert schemas.resolve(valid, expected="self.Pattern").to_python() == "١٢\\x"
    for value in ["12x", "dd\\x", "z12\\x", "12\\xz"]:
        with pytest.raises(daml.ResolveError) as caught:
            schemas.resolve(daml.parse(json.dumps(value)), expected="self.Pattern")
        assert caught.value.kind == "SchemaViolation"


def test_verbose_multiline_regex_keeps_final_comment_and_full_matching():
    source = '''!String { pattern: !Regex """
  (?x)a | abc # long alternative continues below
  d # final comment
  """
}'''
    bundle = daml.Bundle.from_sources(
        ".", {"daml.bundle": "{}", "Pattern.daml": source}
    )
    document = daml.parse('"abcd"')
    schemas = bundle.compile(document, expected="self.Pattern")
    assert schemas.resolve(document, expected="self.Pattern").to_python() == "abcd"
    for value in ["abc", "zabcd", "abcdz"]:
        with pytest.raises(daml.ResolveError):
            schemas.resolve(daml.parse(json.dumps(value)), expected="self.Pattern")


def test_multiline_regex_compile_error_keeps_original_utf8_payload_coordinates():
    source = '# 😀 original\n!String { pattern: !Regex """\n  é(?=prefix).*\n  """\n}'
    bundle = daml.Bundle.from_sources(
        ".", {"daml.bundle": "{}", "Pattern.daml": source}
    )
    with pytest.raises(daml.NativeError) as caught:
        bundle.compile(daml.parse('""'), expected="self.Pattern")
    encoded = source.encode("utf-8")
    start, end = caught.value.span
    assert encoded.index("é(?=prefix)".encode()) >= start
    assert end >= encoded.index(b".*") + 2
    assert encoded.index(b'"""') + 3 <= start <= end <= encoded.rindex(b'"""')
    encoded[:start].decode("utf-8")
    encoded[:end].decode("utf-8")


def test_regex_data_assertion_does_not_gain_a_native_wire_kind():
    value = daml.parse('!Regex "\\\\d+"').to_python()
    assert value == "\\d+"
    with pytest.raises(daml.ParseError):
        daml.parse('!Regex "\\d+"')
