import type { Automerge, Heads, ObjID, Patch } from "./wasm_types.js"

import { CLEAR_CACHE, IS_PROXY, OBJECT_ID, STATE, TRACE } from "./constants.js"

import { Doc, PatchCallback } from "./types.js"

export interface InternalState<T> {
  handle: Automerge
  heads: Heads | undefined
  freeze: boolean
  mostRecentPatch: {
    before: Heads | undefined
    after: Heads | undefined
    patches: Patch[]
  }
  patchCallback?: PatchCallback<T>
}

// The functions in this module are the only place which reads the hidden
// metadata that the backend (or a change proxy) attaches to document objects.
// Everything else goes through them so that how this metadata is stored can
// change per backend without touching the rest of the package.

/**
 * Return the metadata attached to an Automerge object (a materialised object
 * or a proxy inside a change callback), or undefined if `value` is not one.
 *
 * For document roots this is the full {@link InternalState}, for nested
 * objects it may only contain the `handle`.
 */
export function _meta<T>(
  value: unknown,
): Partial<InternalState<T>> | undefined {
  if (typeof value !== "object" || value === null) {
    return undefined
  }
  return Reflect.get(value, STATE) as Partial<InternalState<T>> | undefined
}

/**
 * Return the backend handle of the document `value` belongs to, or undefined
 * if `value` is not an Automerge object.
 */
export function _handle(value: unknown): Automerge | undefined {
  return _meta(value)?.handle
}

export function _state<T>(doc: Doc<T>, checkroot = true): InternalState<T> {
  if (typeof doc !== "object") {
    throw new RangeError("must be the document root")
  }
  const state = _meta(doc) as InternalState<T>
  if (
    state === undefined ||
    state == null ||
    (checkroot && _obj(doc) !== "_root")
  ) {
    throw new RangeError("must be the document root")
  }
  return state
}

export function _clear_cache<T>(doc: Doc<T>): void {
  Reflect.set(doc, CLEAR_CACHE, true)
}

export function _trace<T>(doc: Doc<T>): string | undefined {
  return Reflect.get(doc, TRACE) as string
}

export function _obj<T>(doc: Doc<T>): ObjID | null {
  if (!(typeof doc === "object") || doc === null) {
    return null
  }
  return Reflect.get(doc, OBJECT_ID) as ObjID
}

export function _is_proxy<T>(doc: Doc<T>): boolean {
  return !!Reflect.get(doc, IS_PROXY)
}
