use daml::{ErrorOrigin, NativeError, Origin, ParseError, ResolveError, SemanticError, Span};
use js_sys::{Object, Reflect};
use wasm_bindgen::prelude::*;

pub(crate) fn set(target: &JsValue, key: &str, value: &JsValue) -> Result<(), JsValue> {
    if Reflect::set(target, &JsValue::from_str(key), value)? {
        Ok(())
    } else {
        Err(js_sys::Error::new("cannot define SDK output property").into())
    }
}

pub(crate) fn error(category: &str, kind: &str, message: &str, pointer: Option<&str>) -> JsValue {
    let error = js_sys::Error::new(message);
    error.set_name("DamlError");
    let result = (|| {
        set(error.as_ref(), "category", &JsValue::from_str(category))?;
        set(error.as_ref(), "kind", &JsValue::from_str(kind))?;
        if let Some(pointer) = pointer {
            set(error.as_ref(), "pointer", &JsValue::from_str(pointer))?;
        }
        Ok::<_, JsValue>(())
    })();
    match result {
        Ok(()) => error.into(),
        Err(cause) => cause,
    }
}

pub(crate) fn span(value: Span) -> Result<JsValue, JsValue> {
    let out = Object::new();
    set(&out, "start", &JsValue::from_f64(value.start as f64))?;
    set(&out, "end", &JsValue::from_f64(value.end as f64))?;
    Ok(out.into())
}

pub(crate) fn origin(value: Origin<'_>) -> Result<JsValue, JsValue> {
    let out = Object::new();
    match value {
        Origin::Input(value) => {
            set(&out, "kind", &JsValue::from_str("input"))?;
            set(&out, "span", &span(value)?)?;
        }
        Origin::Semantic => {
            set(&out, "kind", &JsValue::from_str("semantic"))?;
        }
        Origin::Default(value) => {
            set(&out, "kind", &JsValue::from_str("default"))?;
            set(&out, "resource", &JsValue::from_str(&value.resource))?;
            set(&out, "pointer", &JsValue::from_str(&value.pointer))?;
        }
    }
    Ok(out.into())
}

fn position(source: &str, offset: usize) -> Result<JsValue, JsValue> {
    let mut line = 0_u32;
    let mut character = 0_u32;
    for value in source[..offset].chars() {
        if value == '\n' {
            line += 1;
            character = 0;
        } else {
            character += value.len_utf16() as u32;
        }
    }
    let out = Object::new();
    set(&out, "line", &JsValue::from(line))?;
    set(&out, "character", &JsValue::from(character))?;
    Ok(out.into())
}

fn attach_span(out: &JsValue, value: Span, source: Option<&str>) -> Result<(), JsValue> {
    set(out, "span", &span(value)?)?;
    if let Some(source) = source {
        let range = Object::new();
        set(&range, "start", &position(source, value.start)?)?;
        set(&range, "end", &position(source, value.end)?)?;
        set(out, "range", &range)?;
    }
    Ok(())
}

pub(crate) fn parse(value: ParseError, source: &str) -> JsValue {
    let out = error(
        "parse",
        &format!("{:?}", value.kind),
        &value.to_string(),
        None,
    );
    attach_span(&out, value.span, Some(source))
        .err()
        .unwrap_or(out)
}

pub(crate) fn native(value: NativeError) -> JsValue {
    let out = error(
        "native",
        &format!("{:?}", value.kind),
        &value.to_string(),
        None,
    );
    let attached = (|| {
        if let Some(path) = &value.path {
            set(&out, "path", &JsValue::from_str(&path.to_string_lossy()))?;
        }
        if let Some(reference) = &value.reference {
            set(&out, "reference", &JsValue::from_str(reference))?;
        }
        if let Some(value) = value.span {
            attach_span(&out, value, None)?;
        }
        Ok::<_, JsValue>(())
    })();
    attached.err().unwrap_or(out)
}

pub(crate) fn resolve(value: ResolveError, source: &str) -> JsValue {
    let out = error(
        "resolve",
        &format!("{:?}", value.kind),
        &value.message,
        Some(&value.path),
    );
    let attached = (|| {
        match &value.origin {
            ErrorOrigin::Input(value) => attach_span(&out, *value, Some(source))?,
            ErrorOrigin::Semantic => {
                set(&out, "origin", &origin(Origin::Semantic)?)?;
            }
            ErrorOrigin::Default(value) => {
                set(&out, "origin", &origin(Origin::Default(value))?)?;
            }
        }
        if let Some(value) = &value.schema {
            set(&out, "schema", &origin(Origin::Default(value))?)?;
        }
        Ok::<_, JsValue>(())
    })();
    attached.err().unwrap_or(out)
}

pub(crate) fn semantic(value: SemanticError) -> JsValue {
    error(
        "semantic",
        &format!("{:?}", value.kind),
        &value.message,
        Some(&value.pointer),
    )
}
