/**
 * Compile-time checks that the backend interface declared in `backend.ts`
 * stays structurally equivalent to what `wasm-bindgen` generates for
 * `automerge-wasm`.
 *
 * `API`, `Automerge` and `SyncState` used to be taken directly from the
 * generated `wasm_types.d.ts`, and they are visible from the public API
 * through the hidden `use()` and `getBackend()`. The JS package now declares
 * them itself, and these assertions make sure that the two do not drift
 * apart: if a method is added to, removed from or changed in one of them,
 * type checking fails here.
 *
 * This module contains no runtime code.
 *
 * @module
 * @hidden
 */

import type * as Backend from "./backend.js"
import type * as Wasm from "./wasm_types.js"

/** `true` if `A` and `B` are mutually assignable, `false` otherwise. */
type Equivalent<A, B> = [A] extends [B]
  ? [B] extends [A]
    ? true
    : false
  : false

/** Fails to compile unless `T` is `true`. */
type Assert<T extends true> = T

export type AutomergeMatchesWasm = Assert<
  Equivalent<Backend.Automerge, Wasm.Automerge>
>
export type SyncStateMatchesWasm = Assert<
  Equivalent<Backend.SyncState, Wasm.SyncState>
>
export type ApiMatchesWasm = Assert<Equivalent<Backend.API, Wasm.API>>

// The generated module must also be usable as a backend as it is.
type WasmModule =
  typeof import("./wasm_bindgen_output/nodejs/automerge_wasm.cjs")
export type WasmModuleIsBackend = Assert<
  WasmModule extends Backend.API ? true : false
>
