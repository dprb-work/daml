//! Local filesystem representation of already parsed LSP URIs.

use lsp_types::Uri;
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub(super) const MAX_URI_BYTES: usize = 32_768;

pub(super) fn file_path(uri: &Uri) -> Result<PathBuf, &'static str> {
    if uri.as_str().len() > MAX_URI_BYTES {
        return Err("local file URI exceeds the editor limit");
    }
    if !uri
        .scheme()
        .is_some_and(|scheme| scheme.as_str().eq_ignore_ascii_case("file"))
        || uri.authority().is_some_and(|authority| {
            !authority.as_str().is_empty() && !authority.as_str().eq_ignore_ascii_case("localhost")
        })
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        return Err("expected an absolute local file URI without query or fragment");
    }
    // Uri owns RFC parsing/escape admission; reuse its component decoder rather
    // than introducing a second percent-escape grammar at the filesystem boundary.
    let component = uri.path();
    let bytes = component.as_estr().decode().into_bytes();
    if bytes.contains(&0) {
        return Err("local file URI contains a null byte");
    }
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStringExt;
        PathBuf::from(std::ffi::OsString::from_vec(bytes.into_owned()))
    };
    #[cfg(not(unix))]
    let path = {
        let mut text = String::from_utf8(bytes.into_owned())
            .map_err(|_| "local file URI cannot represent this platform path")?;
        #[cfg(windows)]
        {
            if text.starts_with("//") || text.starts_with("/\\") {
                return Err("network file paths are not local editor contexts");
            }
            if text.as_bytes().get(2) == Some(&b':') && text.starts_with('/') {
                text.remove(0);
            }
        }
        PathBuf::from(text)
    };
    if !path.is_absolute() {
        return Err("expected an absolute local file URI");
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if !matches!(path.components().next(), Some(Component::Prefix(prefix))
            if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
        {
            return Err("network/device file paths are not local editor contexts");
        }
    }
    Ok(path)
}

pub(super) fn path_uri(path: &Path) -> Result<Uri, &'static str> {
    if !path.is_absolute() {
        return Err("source location is not an absolute local path");
    }
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes()
    };
    #[cfg(not(unix))]
    let normalized = {
        let text = path
            .to_str()
            .ok_or("source path has no URI representation")?;
        if text.len() > MAX_URI_BYTES {
            return Err("source URI exceeds the editor limit");
        }
        #[cfg(windows)]
        {
            let text = text.strip_prefix("\\\\?\\").unwrap_or(text);
            if text.starts_with("\\\\") || text.starts_with("//") {
                return Err("network source paths are not local editor contexts");
            }
            format!("/{}", text.replace('\\', "/"))
        }
        #[cfg(not(windows))]
        text.to_owned()
    };
    #[cfg(not(unix))]
    let bytes = normalized.as_bytes();
    if bytes.contains(&0) {
        return Err("source path contains a null byte");
    }
    let size = bytes
        .iter()
        .try_fold(7usize, |size, byte| {
            size.checked_add(if uri_byte(*byte) { 1 } else { 3 })
        })
        .filter(|size| *size <= MAX_URI_BYTES)
        .ok_or("source URI exceeds the editor limit")?;
    let mut text = String::with_capacity(size);
    text.push_str("file://");
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for &byte in bytes {
        if uri_byte(byte) {
            text.push(char::from(byte));
        } else {
            text.push('%');
            text.push(char::from(HEX[usize::from(byte >> 4)]));
            text.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    let input = serde::de::value::StringDeserializer::<serde::de::value::Error>::new(text);
    Ok(Uri::deserialize(input).expect("absolute percent-encoded local paths form a file URI"))
}

fn uri_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"/-._~:".contains(&byte)
}
