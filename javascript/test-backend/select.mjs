// Selects the backend the test suite in `test/` runs against, without
// changing the tests themselves.
//
// Load it with `node --import` (see the `test:*` scripts in package.json).
// It reads `AUTOMERGE_BACKEND`, which is one of:
//
// - `wasm` (the default): nothing is changed, the tests run exactly as they
//   do under `npm test`.
// - `node-api`: imports of `src/entrypoints/fullfat_node.js` from the tests
//   are redirected to `src/entrypoints/node_api.js`, and that entry point is
//   loaded before any test, so tests which only import `src/index.js` see the
//   same backend.
import { register } from "node:module"

const BACKENDS = {
  wasm: null,
  "node-api": "node_api.js",
}

const backend = process.env.AUTOMERGE_BACKEND ?? "wasm"
if (!(backend in BACKENDS)) {
  throw new Error(
    `unknown AUTOMERGE_BACKEND ${JSON.stringify(backend)}, expected one of ${Object.keys(BACKENDS).join(", ")}`,
  )
}

const entrypoint = BACKENDS[backend]
if (entrypoint !== null) {
  const entrypointUrl = new URL(
    `../src/entrypoints/${entrypoint}`,
    import.meta.url,
  )
  register("./resolve.mjs", import.meta.url, {
    data: { entrypoint: entrypointUrl.href },
  })
  await import(entrypointUrl.href)
}
