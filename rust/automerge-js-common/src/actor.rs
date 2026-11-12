//! Parsing of actor ids and authors.

use automerge::{ActorId, Author};

#[derive(Debug, thiserror::Error)]
#[error("could not parse Actor ID as a hex string: {0}")]
pub struct BadActorId(#[from] hex::FromHexError);

// The message is the same as for actor ids, which is what both bindings have
// always reported.
#[derive(Debug, thiserror::Error)]
#[error("could not parse Actor ID as a hex string: {0}")]
pub struct BadAuthor(#[from] automerge::error::InvalidAuthor);

pub fn parse_actor(actor: &str) -> Result<ActorId, BadActorId> {
    Ok(ActorId::from(hex::decode(actor)?))
}

pub fn parse_author(author: String) -> Result<Author<'static>, BadAuthor> {
    Ok(Author::try_from(author)?)
}
