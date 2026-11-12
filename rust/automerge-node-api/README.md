# automerge-node-api

A [Node-API](https://nodejs.org/api/n-api.html) binding to the Rust
implementation of Automerge, built with [napi-rs](https://napi.rs).

It implements the whole low-level API of `automerge-wasm` (every method of
the `Automerge` and `SyncState` handles and every free function), and the
end-to-end tests in `javascript/test` pass against it unchanged. It is
wired into the JS package as an additional, non-default entry point,
`@automerge/automerge/node-api`. See
https://github.com/automerge/automerge/pull/1099 for background.

Code with no dependency on a JS engine is shared with `automerge-wasm`
through [`automerge-js-common`](../automerge-js-common).

## Principles

- **The public JavaScript API does not change.** What `@automerge/automerge`
  exports, and what is observable through it (values, patches, error classes
  and messages), is the same whichever binding is underneath. The end-to-end
  tests in `javascript/test`, which exercise the public API, must pass
  unchanged against both bindings.
- **Everything below the public API is in scope.** That includes the
  low-level `API` interface (`javascript/src/low_level.ts`), the internals of
  the JS package and how it divides work with the native side.
- **The WebAssembly binding stays.** `automerge-wasm` remains a first-class
  binding built on `wasm-bindgen`, and the public API of the published
  `@automerge/automerge-wasm` package stays unbroken. Bridging WebAssembly
  through Node-API (e.g. via napi-rs' own wasm target) is not something we
  are pursuing for now.
- **Aligned where it makes sense, not at all costs.** Code with no dependency
  on a JS engine should live in one place and be used by both bindings. Code
  that builds or reads JS values should use the primitives of its binding
  generator idiomatically. When that pulls the two bindings apart, we
  abstract on the JS side and let each binding provide its own
  implementation, rather than forcing one binding into the shape of the
  other.

## Where the bindings meet the JS package

The JS package selects its backend through `UseApi` in
`javascript/src/low_level.ts`. That seam assumed `wasm-bindgen` in a few
places. Their current state:

- **Materialisation.** `implementation.ts` calls `handle.materialize`,
  `handle.applyPatches` and `handle.applyAndReturnPatches` and expects fully
  built, optionally frozen documents carrying hidden metadata. This is the
  main candidate for an internal abstraction with a per-binding
  implementation. *Not done yet:* for parity, this binding implements the
  same contract natively (`src/export.rs`, ported from `export_cache.rs` and
  the patch code in `automerge-wasm`'s `interop.rs`).
- **Object metadata.** `constants.ts` defines `STATE`
  (`Symbol.for("_am_meta")`) and `OBJECT_ID` (`Symbol.for("_am_objectId")`),
  which the JS package reads from objects that the binding wrote them onto.
  *Partly resolved:* every read now goes through `_meta`, `_handle`, `_obj`
  and `_state` in `internal_state.ts`, so how the metadata is stored can
  change per backend in one place. Both bindings still store it as symbol
  properties.
- **Handle identity.** `proxies.ts` compared `handle.__wbg_ptr`, a
  `wasm-bindgen` internal, to tell whether a value belongs to the same
  document. *Resolved:* it compares the handles themselves by identity.
- **Types.** `wasm_types.d.ts` is copied from the `wasm-bindgen` output by
  `scripts/build.mjs` and was imported throughout `javascript/src`.
  *Resolved:* see [Types](#types) below.
- **Loading.** `initializeWasm` and the files in `src/entrypoints` are about
  instantiating a wasm module. *Resolved for Node.js:*
  `src/entrypoints/node_api.ts` loads this binding synchronously through
  `src/node_api_backend.ts`, a thin adapter. `initializeWasm` and friends
  are unchanged and still refer to wasm.

## Building and testing

```
yarn install
yarn build:debug   # or `yarn build` for a release build
yarn test
```

To run the JS package's tests against this binding (after building the JS
package as usual, which also builds the wasm binding whose typings it uses):

```
cd ../../javascript
npm run test:node-api                         # javascript/test, unchanged
node scripts/build.mjs -s compile-node-api    # emit dist/mjs/entrypoints/node_api.js
```

`npm test` selects its backend through `test-backend/select.mjs`, which
reads `AUTOMERGE_BACKEND` (`wasm` by default). Run the tests against a
release build: debug builds of `automerge` enable `debug_assert!`s, and one
of them (`validate_op_order` in `automerge/src/automerge/apply.rs`) fails
for the "can load a doc without checking the heads" test, which loads a
document with mismatching heads using `unchecked: true`. The wasm binding is
always built in release mode, so it never sees this.

`scripts/ci/node_api_tests` (the `node_api_tests` CI job) does all of the
above.

## WebAssembly workarounds to revisit

The existing binding is shaped in places by constraints of WebAssembly and
`wasm-bindgen`. When porting, the first goal is parity, so these are ported
as they are. They are listed here so we can come back and use what Node-API
offers instead. Entries are candidates found by reading the code; none of
them have been measured yet.

| Area | What `automerge-wasm` does | Why | What Node-API could do | Status here |
| --- | --- | --- | --- | --- |
| Materialize key strings | `ExportCache::ensure_key` / `ensure_datatype` keep a `HashMap` of `JsString`s so each map key is only encoded once per export (`src/export_cache.rs`) | Every Rust `&str` crossing into JS is copied out of linear memory and decoded | Create property keys directly (`napi_create_string_utf8`, or `node_api_create_property_key_utf16` for internalised keys) and re-measure whether the cache is still needed | Keys are set with `napi_set_named_property`, without a cache. Datatype strings are still cached. Not measured |
| Materialize / patch application | Objects are built one `Reflect::set` / `Object.defineProperty` / `Object.freeze` call at a time, each one a call out of wasm into JS glue | No direct access to the engine | Direct engine calls (`napi_set_named_property`, `napi_define_properties` to set several hidden props in one call, `napi_object_freeze`) | Done: direct calls, and the three hidden properties of an object are defined in one `napi_define_properties` call. Splicing arrays still goes through `Array.prototype.splice` (via `Reflect.apply`), and shallow copies through `Array.from` / `Object.assign`, as in wasm |
| Object metadata | Object id, datatype and meta are stored on every materialised object as non-enumerable `Symbol.for("_am_objectId")`, `_am_datatype_`, `_am_raw_value_`, `_am_meta` properties and read back in `unwrap_object` | The only way to associate Rust-side data with a JS object | `napi_wrap` / `napi_type_tag_object` could attach native data. The JS package reads `_am_objectId` and `_am_meta` too, so this needs the lookup in `internal_state.ts` to go through a per-binding implementation | Ported as is. The JS-side lookups now go through functions, so this can be changed next |
| Binary data | `Uint8Array::to_vec()` on the way in and `Uint8Array::from(slice)` on the way out (`load`, `save`, changes, sync messages, byte scalars) | JS cannot hold references into linear memory, which may move when it grows | Borrow the incoming `ArrayBuffer` for the duration of the call, and hand ownership of an outgoing `Vec<u8>` to an external `ArrayBuffer`/`Buffer` without copying | Done for method arguments (napi-rs `Uint8Array` borrows the JS buffer) and for returned `Vec<u8>`s (`save`, `saveIncremental`, sync messages, ...). Byte scalars and change bytes borrowed from the document are still copied |
| Memory ceiling | 32-bit linear memory, 4GB at most | wasm32 | None needed. This is one of the motivations for the binding, and means sizes should not be assumed to fit in `u32` | Not reviewed. Lengths and indices are passed as `f64`/`u32` as in wasm |
| Lifetime of handles | `Automerge` and `SyncState` must be released with `.free()`; `javascript/src/implementation.ts` calls it explicitly | wasm-bindgen objects are not collected by the JS GC | napi-rs classes are finalised by the GC. `free()` has to keep existing on the low-level API, but can become a no-op or an eager release | `free()` is a no-op. `[Symbol.dispose]` is added by the JS adapter |
| Untyped arguments | Most methods take `JsValue` and validate by hand; 49 of them are `skip_typescript` with types written in a `typescript_custom_section` | `wasm-bindgen` cannot express optional or union parameters | napi-rs can express `Option<T>` and `Either<A, B>` and generates the `.d.ts`, so this binding should use typed parameters. It does not need to generate the same types as `wasm-bindgen`, see [Types](#types) | Done for ids, props, heads, options, numbers, strings and byte arrays. Values, objects being inserted, hydrate values and option bags with several shapes are still read as `Unknown`/`Object`, because what is accepted depends on runtime type checks (Date, Uint8Array, BigInt, registered datatypes) |
| Error classes | Each error enum is converted to a JS `RangeError`/`Error` by hand | `wasm-bindgen` only throws what it is given | napi-rs throws plain `Error` by default, so the binding needs explicit conversions to keep the same classes (see the TODO in `simple.test.ts`) | Done: `src/error.rs` throws each error as the class `automerge-wasm` uses (including the errors wasm throws as plain strings) with the same message, and returns `PendingException` to napi-rs. Errors from napi-rs' own argument conversion (a wrongly typed argument) are napi-rs' `Error`s and differ from wasm's messages |
| Start-up | Async instantiation (`initializeWasm`, base64 / bundler / workerd entrypoints in `javascript/src/entrypoints`), plus an `on_start` abort hook because a trapped instance is unusable afterwards | Module loading and trap semantics of wasm | Synchronous `require`, and a Rust panic can be caught and thrown as a normal exception | Done: synchronous `require`, and every export is generated with napi-rs' `catch_unwind`, so a panic is thrown as an `Error`. Note that wasm throws a `PanicError` (a wasm-bindgen class) instead |
| Text indexing | `utf16-indexing` feature and `TextEncoding::Utf16CodeUnit` | Not a wasm limitation: JS strings are UTF-16 | Keep. This binding must use the same setting to stay compatible | Kept |

Found while porting, and kept identical in both bindings because they are
observable through the public API:

- **Every `uint` is exported as a `BigInt`.** `SAFE_UINT` was written as
  `0..(Number::MIN_SAFE_INTEGER as u64)`, which is `0..0`. This looks
  unintended. It now lives in `automerge-js-common/src/numbers.rs` with a
  comment.
- **A JS number without a datatype is an `int` only if it is at least 1.**
  `0` and negative whole numbers become `f64`
  (`automerge_js_common::numbers::number_to_scalar`).
- **Some errors are thrown as plain strings**, not `Error` objects: invalid
  object ids in `marks`/`marksAt`, invalid heads in a few methods,
  `joinBlock`, `decodeSyncState`, `importSyncState`, `encodeSyncMessage`,
  invalid datatypes and some materialisation errors.
- **`decodeChange`, `getDecodedChangeByHash` and `readBundle`** return what
  `serde_wasm_bindgen`'s JSON compatible serializer produces in wasm. Here
  the change is serialised with `serde_json` and parsed with `JSON.parse`,
  which gives the same values; `encodeChange` likewise reads its argument
  with `JSON.stringify`, as wasm's `into_serde` does.

Limitations of napi-rs met along the way:

- `get_all_property_names` takes a single `KeyFilter`, so own enumerable
  string keys are read with `KeyFilter::Enumerable` and symbol keys skipped
  by hand.
- `JsStringUtf16` may include a terminating NUL; `interop::utf16` truncates
  to `utf16_len()`.

## Types

Three kinds of types come out of `wasm_types.d.ts` today, and only the first
is part of the public API:

- **Data shapes**, such as `Patch`, `Mark`, `Cursor`, `Span`, `Heads` and
  `DecodedChange`. `javascript/src` re-exports these, so they are public.
  They are not generated from Rust signatures though: they are hand-written
  TypeScript in a `typescript_custom_section` string in
  `automerge-wasm/src/lib.rs`.
- **The `API` interface and the `Automerge` handle class.** These are
  exported from the JS package, but only through `use()` and `getBackend()`,
  which are both marked `@hidden`.
- **Method signatures generated by `wasm-bindgen`**, which only reach users
  through the handle class above.

The direction we want is for the JS package to own its types rather than for
both generators to produce identical output. Where that stands:

- The public data shapes move into `javascript/src` as ordinary TypeScript.
  This is mostly relocating what is already hand-written. *Not done, by
  choice:* they stay in `automerge-wasm/src/lib.rs` (the low-cost option
  below), and `javascript/src/backend.ts` is the only module in
  `javascript/src` that imports them from `wasm_types.d.ts`.
- The JS package declares the internal backend interface it needs. Each
  binding is checked against it at compile time where it is plugged in, with
  a thin adapter where the generated shape is not a direct fit. *Done:*
  `API`, `Automerge` and `SyncState` are declared in
  `javascript/src/backend.ts`. `javascript/src/wasm_backend_check.ts`
  asserts that they are structurally equivalent to what `wasm-bindgen`
  generates, and `UseApi` checks the wasm module against them.
  `javascript/src/node_api_backend.ts` checks this binding against them,
  without casts. The `.d.ts` napi-rs generates for this binding refers to
  the data shapes by name; `dts-header.d.ts` declares them, opaque (`any`)
  except for the simple aliases, so that they are not copied. The check
  therefore covers method names, arities and primitive types, but not the
  data shapes of this binding.
- Each binding is then free to use whatever its generator produces
  idiomatically.

Two constraints on that:

- **Structural equivalence is enough.** Types that are visible from the JS
  package, including the hidden `API` and the return type of `getBackend()`,
  may change name or origin as long as they stay structurally equivalent to
  what is exported today.
- **The public API of `@automerge/automerge-wasm` stays unbroken.** That
  package is published on its own and keeps exporting the same types. If the
  data shapes move, it has to keep providing them, for example by continuing
  to emit them or by re-exporting them, and the two copies must not drift.

## Possible future strategy: lazy materialisation

Instead of building a whole document eagerly, a read could return a `Proxy`
whose handlers call into native code when a property is accessed. This would
save memory and work when only a small part of a large document is read. The
JS package already does this inside `change()` (`rootProxy` in
`javascript/src/proxies.ts`), and it is not specific to Node-API: a read-only
variant would work over either binding.

It is postponed, and would be an opt-in addition to the public API rather
than a replacement for the default, because a plain object tree is
observable today:

- `Object.isFrozen(doc)` is asserted by `javascript/test/basic_test.ts` when
  `freeze` is on.
- `structuredClone(doc)` and `postMessage(doc)` throw on a `Proxy`.
- Patch application leaves untouched subtrees as the same object, which
  consumers rely on for memoisation. A lazy proxy would need its own cache to
  preserve that.
- A `Doc` is an immutable view at fixed heads while the native document keeps
  changing, so every lazy read is a read at pinned heads and every old
  snapshot keeps the native document alive.
- Each property access becomes a proxy trap plus a native call, so code that
  walks a whole document gets slower.

What we do now is keep this possible: the internal materialiser interface
should not assume that the result is an eagerly built plain object. In
particular, object id and state lookups should go through functions rather
than reading symbols off the object directly. The cost of eager
materialisation on large documents has not been measured yet, and that
number should decide the priority of this.
