// Module resolution hook registered by `select.mjs`. Redirects the default
// Node.js entry point to the entry point of the selected backend.
let entrypoint

export function initialize(data) {
  entrypoint = data.entrypoint
}

export async function resolve(specifier, context, nextResolve) {
  const resolved = await nextResolve(specifier, context)
  if (/\/src\/entrypoints\/fullfat_node\.(js|ts)$/.test(resolved.url)) {
    return nextResolve(entrypoint, context)
  }
  return resolved
}
