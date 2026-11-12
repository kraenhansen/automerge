//! Parsing of change hashes and heads.

use automerge::ChangeHash;

#[derive(Debug, thiserror::Error)]
pub enum BadChangeHash {
    #[error("change hash was not a string")]
    NotString,
    #[error(transparent)]
    Parse(#[from] automerge::ParseChangeHashError),
}

#[derive(Debug, thiserror::Error)]
pub enum BadChangeHashes {
    #[error("the change hashes were not an array of strings")]
    NotArray,
    #[error("could not decode hash {0}: {1}")]
    BadElem(usize, BadChangeHash),
}

/// Parse one change hash. `None` stands for a JS value which was not a
/// string.
pub fn parse_change_hash(hash: Option<&str>) -> Result<ChangeHash, BadChangeHash> {
    match hash {
        Some(s) => Ok(s.parse()?),
        None => Err(BadChangeHash::NotString),
    }
}

/// Parse the elements of a JS array of change hashes. `None` elements stand
/// for JS values which were not strings.
pub fn parse_heads<I, S>(hashes: I) -> Result<Vec<ChangeHash>, BadChangeHashes>
where
    I: IntoIterator<Item = Option<S>>,
    S: AsRef<str>,
{
    hashes
        .into_iter()
        .enumerate()
        .map(|(i, h)| {
            parse_change_hash(h.as_ref().map(AsRef::as_ref))
                .map_err(|e| BadChangeHashes::BadElem(i, e))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages() {
        let err = parse_heads([Some("abc")]).unwrap_err();
        assert!(err.to_string().starts_with("could not decode hash 0: "));
        let err = parse_heads::<_, &str>([None]).unwrap_err();
        assert_eq!(
            err.to_string(),
            "could not decode hash 0: change hash was not a string"
        );
    }
}
