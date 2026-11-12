/**
 * The interface between this package and the native binding underneath it.
 *
 * There are two kinds of types in here:
 *
 * - **Data shapes** such as {@link Patch}, {@link Mark} or {@link Heads}.
 *   These are hand-written TypeScript in the `typescript_custom_section` of
 *   `rust/automerge-wasm/src/lib.rs` and reach this package through the
 *   copied `wasm_types.d.ts`. This module is the only place in `src` which
 *   imports them from there, so that where they come from can change without
 *   touching the rest of the package.
 * - **The backend interface**, i.e. {@link API}, {@link Automerge} and
 *   {@link SyncState}. These are declared here, by the JS package, rather
 *   than taken from whatever a binding generator emits. A binding is checked
 *   against them at compile time where it is plugged in (see `UseApi` in
 *   `low_level.ts`). `wasm_backend_check.ts` additionally checks that they
 *   stay structurally equivalent to the classes `wasm-bindgen` generates, as
 *   they are visible (through the hidden `use()` and `getBackend()`) from the
 *   public API.
 *
 * @module
 * @hidden
 */

import type {
  Actor,
  Author,
  Change,
  ChangeMetadata,
  ChangeToEncode,
  Commit,
  Cursor,
  CursorPosition,
  Datatype,
  DecodedBundle,
  DecodedChange,
  DecodedSyncMessage,
  DiffOptions,
  Fragment,
  FragmentLevelRange,
  FragmentMeta,
  FullValue,
  FullValueWithId,
  Hash,
  Heads,
  InitOptions,
  JsSyncState,
  LoadOptions,
  Mark,
  MarkRange,
  MarkSet,
  MaterializeValue,
  MoveCursor,
  ObjID,
  ObjInfo,
  ObjType,
  Patch,
  Prop,
  Span,
  Stats,
  SyncMessage,
  UpdateSpansConfig,
  Value,
  WasmReleaseInfo,
} from "./wasm_types.js"

// The data shapes, re-exported for the rest of the package.
export type {
  Actor,
  Author,
  ObjID,
  Change,
  SyncMessage,
  FragmentLevelRange,
  Prop,
  Hash,
  Heads,
  ScalarValue,
  Value,
  MaterializeValue,
  MapObjType,
  ObjInfo,
  Span,
  ListObjType,
  ObjType,
  FullValue,
  Cursor,
  CursorPosition,
  MoveCursor,
  FullValueWithId,
  ObjTypeName,
  Datatype,
  SyncHave,
  DecodedSyncMessage,
  DecodedChange,
  ChangeMetadata,
  FragmentMeta,
  Commit,
  Fragment,
  ChangeToEncode,
  Op,
  PatchValue,
  Patch,
  PutPatch,
  MarkSet,
  MarkPatch,
  MarkRange,
  UnmarkPatch,
  IncPatch,
  DelPatch,
  SpliceTextPatch,
  InsertPatch,
  ConflictPatch,
  Mark,
  LoadOptions,
  DiffOptions,
  InitOptions,
  JsSyncState,
  DecodedBundle,
  Stats,
  WasmReleaseInfo,
  UpdateSpansConfig,
} from "./wasm_types.js"

/**
 * A handle to a native document.
 */
export interface Automerge {
  free(): void
  [Symbol.dispose](): void

  // Lifecycle
  anonymize(): Automerge
  clone(actor?: string | null): Automerge
  fork(actor?: string, heads?: Heads): Automerge
  merge(other: Automerge): Heads
  enableFreeze(enable: boolean): boolean
  registerDatatype(
    datatype: string,
    construct: Function, // eslint-disable-line @typescript-eslint/no-unsafe-function-type
    deconstruct: (arg: any) => any | undefined, // eslint-disable-line @typescript-eslint/no-explicit-any
  ): void
  dump(): void
  stats(): Stats

  // Transactions
  pendingOps(): number
  commit(message?: string | null, time?: number | null): Hash | null
  emptyChange(message?: string | null, time?: number | null): Hash
  rollback(): number
  isolate(heads: Heads): void
  integrate(): void

  // Reads
  get(obj: ObjID, prop: Prop, heads?: Heads): Value | undefined
  getWithType(obj: ObjID, prop: Prop, heads?: Heads): FullValue | null
  getAll(obj: ObjID, arg: Prop, heads?: Heads): FullValueWithId[]
  keys(obj: ObjID, heads?: Heads): string[]
  length(obj: ObjID, heads?: Heads): number
  objInfo(obj: ObjID, heads?: Heads): ObjInfo
  text(obj: ObjID, heads?: Heads): string
  spans(obj: ObjID, heads?: Heads): Span[]
  marks(obj: ObjID, heads?: Heads): Mark[]
  marksAt(obj: ObjID, index: number, heads?: Heads): MarkSet
  getBlock(
    obj: ObjID,
    index: number,
    heads?: Heads,
  ): { [key: string]: MaterializeValue } | null
  getCursor(
    obj: ObjID,
    position: CursorPosition,
    heads?: Heads,
    move?: MoveCursor,
  ): Cursor
  getCursorPosition(obj: ObjID, cursor: Cursor, heads?: Heads): number

  // Writes
  put(obj: ObjID, prop: Prop, value: Value, datatype?: Datatype): void
  putObject(obj: ObjID, prop: Prop, value: ObjType): ObjID
  putObjectFromHydrate(obj: ObjID, prop: Prop, value: any): ObjID // eslint-disable-line @typescript-eslint/no-explicit-any
  insert(obj: ObjID, index: number, value: Value, datatype?: Datatype): void
  insertObject(obj: ObjID, index: number, value: ObjType): ObjID
  insertObjectFromHydrate(obj: ObjID, index: number, value: any): ObjID // eslint-disable-line @typescript-eslint/no-explicit-any
  push(obj: ObjID, value: Value, datatype?: Datatype): void
  pushObject(obj: ObjID, value: ObjType): ObjID
  splice(
    obj: ObjID,
    start: number,
    delete_count: number,
    text?: string | Array<Value>,
  ): void
  spliceFromHydrate(obj: ObjID, index: number, del: number, values: any): void // eslint-disable-line @typescript-eslint/no-explicit-any
  initRootFromHydrate(value: any): void // eslint-disable-line @typescript-eslint/no-explicit-any
  increment(obj: ObjID, prop: Prop, value: number): void
  delete(obj: ObjID, prop: Prop): void

  // Text, marks and blocks
  updateText(obj: ObjID, new_text: string): void
  updateSpans(
    obj: ObjID,
    args: Span[],
    config: UpdateSpansConfig | undefined | null,
  ): void
  mark(
    obj: ObjID,
    range: MarkRange,
    name: string,
    value: Value,
    datatype?: Datatype,
  ): void
  unmark(obj: ObjID, range: MarkRange, name: string): void
  splitBlock(
    obj: ObjID,
    index: number,
    block: { [key: string]: MaterializeValue },
  ): void
  joinBlock(obj: ObjID, index: number): void
  updateBlock(
    obj: ObjID,
    index: number,
    block: { [key: string]: MaterializeValue },
  ): void

  // Materialisation and patches
  materialize(obj?: ObjID, heads?: Heads, metadata?: unknown): MaterializeValue
  toJS(meta: any): MaterializeValue // eslint-disable-line @typescript-eslint/no-explicit-any
  applyPatches<Doc>(obj: Doc, meta?: unknown): Doc
  applyAndReturnPatches<Doc>(
    obj: Doc,
    meta?: unknown,
  ): { value: Doc; patches: Patch[] }
  diff(before: Heads, after: Heads): Patch[]
  diffPath(
    path: Prop[] | string,
    before: Heads,
    after: Heads,
    options?: DiffOptions,
  ): Patch[]
  diffIncremental(): Patch[]
  updateDiffCursor(): void
  resetDiffCursor(): void

  // Changes, save and load
  save(): Uint8Array
  saveIncremental(): Uint8Array
  saveSince(heads: Heads): Uint8Array
  saveNoCompress(): Uint8Array
  saveAndVerify(): Uint8Array
  saveBundle(hashes: any): Uint8Array // eslint-disable-line @typescript-eslint/no-explicit-any
  loadIncremental(data: Uint8Array): number
  applyChanges(changes: Change[]): void
  getChanges(have_deps: Heads): Change[]
  getChangesMeta(have_deps: Heads): ChangeMetadata[]
  getChangeByHash(hash: Hash): Change | null
  getChangeMetaByHash(hash: Hash): ChangeMetadata | null
  getDecodedChangeByHash(hash: Hash): DecodedChange | null
  getChangesAdded(other: Automerge): Change[]
  getLastLocalChange(): Change | null
  getHeads(): Heads
  getMissingDeps(heads?: Heads): Heads
  topoHistoryTraversal(): Hash[]

  // Sync
  receiveSyncMessage(state: SyncState, message: SyncMessage): void
  generateSyncMessage(state: SyncState): SyncMessage | null
  hasOurChanges(state: SyncState): boolean

  // Fragments and commits
  getFragmentMetadata(levels?: FragmentLevelRange): FragmentMeta[]
  getFragmentMeta(head: Hash): FragmentMeta | null
  bundleFragmentMetadata(fragments: FragmentMeta[]): Uint8Array[]
  addCommits(commits: Commit[]): void
  addFragments(fragments: Fragment[]): void

  // Actors and authors
  getActorId(): Actor
  getAuthor(): Author | null
  setAuthor(author?: string | null): void
  getAuthors(): Author[]
  getAuthorForActor(actor: string): Author | null
  getActorsForAuthor(author: string): Actor[]
  maskAuthor(author: string, heads: any): void // eslint-disable-line @typescript-eslint/no-explicit-any
  revealAuthor(author: string): void
  isAuthorMasked(author: string): boolean
}

/**
 * A handle to native sync state.
 */
export interface SyncState {
  free(): void
  [Symbol.dispose](): void
  clone(): SyncState
  lastSentHeads: Heads
  readonly peerReadOnly: boolean
  readOnly: boolean
  set sentHashes(value: Heads)
  readonly sharedHeads: Heads
}

/**
 * The functions a backend provides.
 */
export interface API {
  create(options?: InitOptions): Automerge
  load(data: Uint8Array, options?: LoadOptions): Automerge
  encodeChange(change: ChangeToEncode): Change
  decodeChange(change: Change): DecodedChange
  initSyncState(): SyncState
  encodeSyncMessage(message: DecodedSyncMessage): SyncMessage
  decodeSyncMessage(msg: SyncMessage): DecodedSyncMessage
  encodeSyncState(state: SyncState): Uint8Array
  decodeSyncState(data: Uint8Array): SyncState
  exportSyncState(state: SyncState): JsSyncState
  importSyncState(state: JsSyncState): SyncState
  readBundle(data: Uint8Array): DecodedBundle
  wasmReleaseInfo(): WasmReleaseInfo
}
