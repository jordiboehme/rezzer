// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Decryption with the standard security handler (ISO 32000-1, 7.6). lopdf computes the key
//! and decrypts single objects; it never sees the file. The encryption dictionary is never
//! inside an object stream, and members of an object stream are not encrypted on their own.

use lopdf::encryption::{DecryptionError, decrypt_object};
use lopdf::{Dictionary, Document, EncryptionState, Object, ObjectId};

use super::store::Store;
use super::xref::Entry;
use crate::{Encryption, OpenError};

/// The key and the object it must not touch.
pub(crate) struct Crypt {
    state: EncryptionState,
    /// The encryption dictionary, which is not encrypted.
    pub skip: ObjectId,
}

impl Crypt {
    /// Decrypts the strings and stream data of one plain object. The `/Contents` of a signature
    /// dictionary is not encrypted (7.6.1) and stays as it is.
    pub fn decrypt(&self, id: ObjectId, obj: &mut Object) -> Result<(), DecryptionError> {
        let mut kept = Vec::new();
        take_signatures(obj, false, &mut kept);
        let result = decrypt_object(&self.state, id, obj);
        let mut kept = kept.into_iter();
        put_signatures(obj, false, &mut kept);
        result
    }
}

fn is_signature(d: &Dictionary) -> bool {
    d.has_type(b"Sig") || d.has_type(b"DocTimeStamp") || d.get(b"ByteRange").is_ok()
}

/// The fields of a signature field whose `/V` is a direct signature dictionary.
fn is_signature_field(d: &Dictionary) -> bool {
    matches!(d.get(b"FT"), Ok(Object::Name(n)) if n == b"Sig")
}

/// Moves the `/Contents` of every signature dictionary inside `obj` out, in walk order.
fn take_signatures(obj: &mut Object, signature: bool, kept: &mut Vec<Object>) {
    match obj {
        Object::Dictionary(d) => take_in(d, signature, kept),
        Object::Stream(s) => take_in(&mut s.dict, signature, kept),
        Object::Array(a) => a.iter_mut().for_each(|o| take_signatures(o, false, kept)),
        _ => {}
    }
}

fn take_in(d: &mut Dictionary, signature: bool, kept: &mut Vec<Object>) {
    if (signature || is_signature(d))
        && let Ok(c) = d.get_mut(b"Contents")
    {
        kept.push(std::mem::replace(c, Object::Null));
    }
    let field = is_signature_field(d);
    for (k, v) in d.iter_mut() {
        if k == b"Contents" && !matches!(v, Object::Dictionary(_) | Object::Array(_)) {
            continue;
        }
        take_signatures(v, field && k == b"V", kept);
    }
}

/// Puts the values [`take_signatures`] moved out back, in the same walk order.
fn put_signatures(obj: &mut Object, signature: bool, kept: &mut impl Iterator<Item = Object>) {
    match obj {
        Object::Dictionary(d) => put_in(d, signature, kept),
        Object::Stream(s) => put_in(&mut s.dict, signature, kept),
        Object::Array(a) => a.iter_mut().for_each(|o| put_signatures(o, false, kept)),
        _ => {}
    }
}

fn put_in(d: &mut Dictionary, signature: bool, kept: &mut impl Iterator<Item = Object>) {
    if (signature || is_signature(d))
        && let Ok(c) = d.get_mut(b"Contents")
        && let Some(v) = kept.next()
    {
        *c = v;
    }
    let field = is_signature_field(d);
    for (k, v) in d.iter_mut() {
        if k == b"Contents" && !matches!(v, Object::Dictionary(_) | Object::Array(_)) {
            continue;
        }
        put_signatures(v, field && k == b"V", kept);
    }
}

/// Sets up decryption when the trailer names an encryption dictionary. The empty password is
/// tried first, then `password`. The password never reaches an error text.
pub(crate) fn setup(
    store: &mut Store,
    trailer: &Dictionary,
    password: Option<&str>,
) -> Result<Option<(Crypt, Encryption)>, OpenError> {
    let Ok(value) = trailer.get(b"Encrypt") else {
        return Ok(None);
    };
    let (id, dict) = match value {
        Object::Reference(id) => {
            if matches!(store.entry(id.0), Some(Entry::Compressed { .. })) {
                return Err(OpenError::Damaged(
                    "the encryption dictionary is inside an object stream".into(),
                ));
            }
            match store.load(*id)? {
                Some(Object::Dictionary(d)) => (*id, d),
                _ => {
                    return Err(OpenError::Damaged(
                        "the encryption dictionary is missing".into(),
                    ));
                }
            }
        }
        Object::Dictionary(d) => ((u32::MAX, 0), d.clone()),
        _ => {
            return Err(OpenError::Damaged(
                "the encryption dictionary is missing".into(),
            ));
        }
    };
    check_handler(&dict)?;
    // lopdf reads the dictionary and the file identifier from a document; this one holds
    // nothing else.
    let mut doc = Document::new();
    doc.objects.insert(id, Object::Dictionary(dict));
    doc.trailer.set("Encrypt", Object::Reference(id));
    if let Ok(file_id) = trailer.get(b"ID") {
        doc.trailer.set("ID", file_id.clone());
    }
    let (pw, empty) = if opens(&doc, "", password)? {
        ("", true)
    } else {
        match password {
            None => return Err(OpenError::NeedsPassword),
            Some(p) if opens(&doc, p, password)? => (p, false),
            Some(_) => return Err(OpenError::WrongPassword),
        }
    };
    let state = EncryptionState::decode(&doc, pw).map_err(|e| lopdf_error(e, password))?;
    let encryption = Encryption {
        method: method(&state),
        empty_user_password: empty,
    };
    Ok(Some((Crypt { state, skip: id }, encryption)))
}

/// Whether `pw` opens the document as user or owner password.
fn opens(doc: &Document, pw: &str, password: Option<&str>) -> Result<bool, OpenError> {
    match doc.authenticate_password(pw) {
        Ok(()) => Ok(true),
        Err(lopdf::Error::Decryption(DecryptionError::IncorrectPassword))
        | Err(lopdf::Error::InvalidPassword) => Ok(false),
        Err(e) => Err(lopdf_error(e, password)),
    }
}

fn lopdf_error(e: lopdf::Error, password: Option<&str>) -> OpenError {
    let text = e.to_string();
    let text = match password {
        Some(p) if !p.is_empty() && text.contains(p) => text.replace(p, "***"),
        _ => text,
    };
    OpenError::UnsupportedEncryption(text)
}

/// Refuses encryption this crate cannot open, before asking for a password.
fn check_handler(dict: &Dictionary) -> Result<(), OpenError> {
    let name = |k: &[u8]| {
        dict.get(k)
            .ok()
            .and_then(|o| o.as_name().ok())
            .unwrap_or(b"")
    };
    let int = |k: &[u8]| dict.get(k).ok().and_then(|o| o.as_i64().ok()).unwrap_or(0);
    let filter = name(b"Filter");
    if filter != b"Standard" {
        return Err(OpenError::UnsupportedEncryption(format!(
            "security handler {}",
            String::from_utf8_lossy(filter)
        )));
    }
    let (v, r) = (int(b"V"), int(b"R"));
    if !matches!(v, 1 | 2 | 4 | 5) || !(2..=6).contains(&r) {
        return Err(OpenError::UnsupportedEncryption(format!(
            "standard security handler version {v} revision {r}"
        )));
    }
    Ok(())
}

fn method(state: &EncryptionState) -> String {
    let bits = state.key_length().unwrap_or(40);
    match state.version() {
        1 => "RC4-40".to_string(),
        2 | 3 => format!("RC4-{bits}"),
        4 | 5 => {
            let name = state.default_stream_filter();
            let cfm = state
                .crypt_filters()
                .get(name)
                .map(|f| f.method().to_vec())
                .unwrap_or_else(|| name.to_vec());
            match cfm.as_slice() {
                b"AESV2" => "AES-128".to_string(),
                b"AESV3" => "AES-256".to_string(),
                b"V2" => format!("RC4-{}", if bits > 40 { bits } else { 128 }),
                b"Identity" => "none".to_string(),
                other => String::from_utf8_lossy(other).into_owned(),
            }
        }
        v => format!("unknown (version {v})"),
    }
}
