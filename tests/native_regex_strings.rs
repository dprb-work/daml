//! Raw native Regex payloads share multiline layout, not string escape decoding.

use daml::parse_definition;

#[test]
fn native_regex_accepts_both_multiline_payload_forms() {
    for delimiter in ["\"\"\"", "'''"] {
        let source = format!("!String {{ pattern: !Regex {delimiter}\n  \\d+\n  {delimiter}\n}}");
        parse_definition(&source).expect("native Regex accepts raw multiline payloads");
    }
}
