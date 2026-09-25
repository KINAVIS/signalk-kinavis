// The built plugin exports what Signal K requires of a Rust library plugin
// and imports nothing beyond the server's `env` and WASI preview 1.
import { readFileSync } from 'node:fs'

const module = new WebAssembly.Module(readFileSync(process.argv[2] ?? 'plugin.wasm'))
const exports = new Set(WebAssembly.Module.exports(module).map((entry) => entry.name))
const required = ['plugin_id', 'plugin_name', 'plugin_schema', 'plugin_start', 'plugin_stop',
  'allocate', 'deallocate', 'delta_handler', 'poll', 'memory']
const missing = required.filter((name) => !exports.has(name))
const foreign = WebAssembly.Module.imports(module)
  .filter((entry) => entry.module !== 'env' && entry.module !== 'wasi_snapshot_preview1')
  .map((entry) => `${entry.module}.${entry.name}`)

if (missing.length > 0 || foreign.length > 0) {
  console.error({ missing, foreign })
  process.exit(1)
}
console.log(`exports: ${[...exports].join(', ')}`)
