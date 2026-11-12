//! Commits passed to `addCommits`.

use automerge::{Change, ChangeHash};

#[derive(Debug, thiserror::Error)]
pub enum BadCommitBytes {
    #[error("bad commit bytes: {0}")]
    Load(#[from] automerge::LoadChangeError),
    #[error("commit input head mismatch: expected {expected}, actual {actual}")]
    HeadMismatch { expected: String, actual: String },
    #[error("commit input parents do not match encoded change dependencies")]
    ParentsMismatch,
}

#[derive(Debug)]
pub struct Commit {
    pub head: ChangeHash,
    pub parents: Vec<ChangeHash>,
    pub bytes: Vec<u8>,
}

impl Commit {
    /// Decode the change in this commit, checking that the metadata lines up
    /// with the bytes.
    ///
    /// This keeps Automerge's API shaped like Subduction's `addCommits`
    /// input while still treating the encoded change as authoritative.
    pub fn into_change(self) -> Result<Change, BadCommitBytes> {
        let change = Change::try_from(self.bytes.as_slice())?;
        if change.hash() != self.head {
            return Err(BadCommitBytes::HeadMismatch {
                expected: self.head.to_string(),
                actual: change.hash().to_string(),
            });
        }
        if change.deps() != self.parents.as_slice() {
            return Err(BadCommitBytes::ParentsMismatch);
        }
        Ok(change)
    }
}
