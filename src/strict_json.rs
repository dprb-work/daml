use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::{collections::HashSet, fmt};

pub(crate) fn parse(source: &str) -> Result<Value, serde_json::Error> {
    // Value's map deserializer overwrites duplicate keys. Check uniqueness first,
    // then let serde_json retain arbitrary-precision numeric lexemes. A separate
    // pass avoids reimplementing its special arbitrary-precision number protocol.
    let mut reader = serde_json::Deserializer::from_str(source);
    UniqueKeys::deserialize(&mut reader)?;
    reader.end()?;
    serde_json::from_str(source)
}

struct UniqueKeys;

impl<'de> Deserialize<'de> for UniqueKeys {
    fn deserialize<D: Deserializer<'de>>(reader: D) -> Result<Self, D::Error> {
        reader.deserialize_any(UniqueVisitor).map(|()| Self)
    }
}

struct UniqueVisitor;

impl<'de> Visitor<'de> for UniqueVisitor {
    type Value = ();

    fn expecting(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str("JSON with unique object members")
    }

    fn visit_unit<E: de::Error>(self) -> Result<(), E> {
        Ok(())
    }
    fn visit_bool<E: de::Error>(self, _: bool) -> Result<(), E> {
        Ok(())
    }
    fn visit_i64<E: de::Error>(self, _: i64) -> Result<(), E> {
        Ok(())
    }
    fn visit_u64<E: de::Error>(self, _: u64) -> Result<(), E> {
        Ok(())
    }
    fn visit_f64<E: de::Error>(self, _: f64) -> Result<(), E> {
        Ok(())
    }
    fn visit_str<E: de::Error>(self, _: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut entries: A) -> Result<(), A::Error> {
        while entries.next_element::<UniqueKeys>()?.is_some() {}
        Ok(())
    }

    fn visit_map<A: MapAccess<'de>>(self, mut entries: A) -> Result<(), A::Error> {
        let mut names = HashSet::new();
        while let Some(name) = entries.next_key::<String>()? {
            if !names.insert(name) {
                return Err(de::Error::custom("duplicate JSON object member"));
            }
            entries.next_value::<UniqueKeys>()?;
        }
        Ok(())
    }
}
