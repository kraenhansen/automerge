//! Resolving object ids and paths as passed from JS.

use automerge::{AutoCommit, AutomergeError, ObjId, ObjType, Prop, ReadDoc, Value, ROOT};

#[derive(Debug, thiserror::Error)]
pub enum ImportObj {
    #[error("obj id was not a string")]
    NotString,
    #[error("invalid path {0}: {1}")]
    InvalidPath(String, Box<ImportPath>),
    #[error("unable to import object id: {0}")]
    BadImport(AutomergeError),
}

#[derive(Debug, thiserror::Error)]
pub enum ImportPath {
    #[error(transparent)]
    Automerge(#[from] AutomergeError),
    #[error("path component {0} ({1}) should be an integer to index a sequence")]
    IndexNotInteger(usize, String),
    #[error("path component {0} ({1}) referenced a nonexistent object")]
    NonExistentObject(usize, String),
    #[error("path did not refer to an object")]
    NotAnObject,
}

#[derive(Debug, thiserror::Error)]
#[error("given property was not a string or integer")]
pub struct InvalidProp;

/// Resolve an object id or a path to an object.
///
/// Valid formats are
///
/// - `123@aabbcc`
/// - `123@aabccc/prop1/prop2/prop3`
/// - `/prop1/prop2/prop3`
pub fn import_obj(doc: &AutoCommit, s: &str) -> Result<(ObjId, ObjType), ImportObj> {
    let mut components = s.split('/');
    let obj = components.next();
    let (id, obj_type) = if obj == Some("") {
        (ROOT, ObjType::Map)
    } else {
        doc.import(obj.unwrap_or_default())
            .map_err(ImportObj::BadImport)?
    };
    import_path(doc, id, obj_type, components)
        .map_err(|e| ImportObj::InvalidPath(s.to_string(), Box::new(e)))
}

pub fn import_path<'a, I: Iterator<Item = &'a str>>(
    doc: &AutoCommit,
    mut obj: ObjId,
    mut obj_type: ObjType,
    components: I,
) -> Result<(ObjId, ObjType), ImportPath> {
    for (i, prop) in components.enumerate() {
        if prop.is_empty() {
            break;
        }
        let is_map = matches!(obj_type, ObjType::Map | ObjType::Table);
        let val = if is_map {
            doc.get(obj, prop)?
        } else {
            let idx = prop
                .parse()
                .map_err(|_| ImportPath::IndexNotInteger(i, prop.to_string()))?;
            doc.get(obj, Prop::Seq(idx))?
        };
        match val {
            Some((Value::Object(t), id)) => {
                obj_type = t;
                obj = id;
            }
            None => return Err(ImportPath::NonExistentObject(i, prop.to_string())),
            _ => return Err(ImportPath::NotAnObject),
        };
    }
    Ok((obj, obj_type))
}

/// Convert a JS number used as a property to a [`Prop`].
///
/// JS numbers are doubles; this truncates like `as usize` (negative numbers
/// and NaN become 0), which is what both bindings have always done.
pub fn number_to_prop(n: f64) -> Prop {
    Prop::Seq(n as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use automerge::transaction::Transactable;

    #[test]
    fn paths() {
        let mut doc = AutoCommit::new();
        let list = doc.put_object(ROOT, "list", ObjType::List).unwrap();
        let map = doc.insert_object(&list, 0, ObjType::Map).unwrap();
        assert_eq!(import_obj(&doc, "/").unwrap(), (ROOT, ObjType::Map));
        assert_eq!(
            import_obj(&doc, "/list/0").unwrap(),
            (map.clone(), ObjType::Map)
        );
        assert_eq!(
            import_obj(&doc, &format!("{}/0", list)).unwrap(),
            (map, ObjType::Map)
        );
        assert_eq!(
            import_obj(&doc, "/list/x").unwrap_err().to_string(),
            "invalid path /list/x: path component 1 (x) should be an integer to index a sequence"
        );
        assert_eq!(
            import_obj(&doc, "/nope").unwrap_err().to_string(),
            "invalid path /nope: path component 0 (nope) referenced a nonexistent object"
        );
    }
}
