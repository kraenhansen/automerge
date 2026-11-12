# automerge-js-common

Code shared by the two JavaScript bindings to Automerge,
[`automerge-wasm`](../automerge-wasm) (`wasm-bindgen`) and
[`automerge-node-api`](../automerge-node-api) (napi-rs).

Only code which has no dependency on a JavaScript engine lives here: parsing
of identifiers, heads, cursors, marks, sync capabilities, commits and
fragment ranges, object id and path lookup, the rules for when an integer is
safe to represent as a JS number, and the error types (and messages) for all
of these. Converting to and from JS values, materialisation and patch
application are done by each binding with the primitives of its own binding
generator.

Error messages are observable through the public API of
`@automerge/automerge`, so they must not change without considering both
bindings.
