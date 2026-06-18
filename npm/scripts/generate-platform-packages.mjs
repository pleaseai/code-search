#!/usr/bin/env node
// Generate the per-platform npm packages from built release assets.
// ADR-0003 / T023. Usage:
//
//   node npm/scripts/generate-platform-packages.mjs <version> <assets-dir>
//
// <assets-dir> holds the csp-<target>[.exe] binaries produced by
// release-rust.yml. For each known target it writes npm/dist/<pkg>/ containing
// a package.json (with os/cpu/libc constraints) and the binary, plus a wrapper
// package.json with pinned optionalDependencies. Publish each with
// `npm publish ./<dir> --provenance --access public`.

import { chmodSync, copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const npmRoot = resolve(here, '..')

// asset = the file name emitted by release-rust.yml; binary = its name inside
// the published package (matches bin/csp.js resolution).
const TARGETS = [
  { pkg: '@pleaseai/csp-darwin-arm64', asset: 'csp-darwin-arm64', binary: 'csp', os: 'darwin', cpu: 'arm64' },
  { pkg: '@pleaseai/csp-darwin-x64', asset: 'csp-darwin-x64', binary: 'csp', os: 'darwin', cpu: 'x64' },
  { pkg: '@pleaseai/csp-linux-x64', asset: 'csp-linux-x64', binary: 'csp', os: 'linux', cpu: 'x64', libc: 'glibc' },
  { pkg: '@pleaseai/csp-linux-arm64', asset: 'csp-linux-arm64', binary: 'csp', os: 'linux', cpu: 'arm64' },
  { pkg: '@pleaseai/csp-linux-x64-musl', asset: 'csp-linux-x64-musl', binary: 'csp', os: 'linux', cpu: 'x64', libc: 'musl' },
  { pkg: '@pleaseai/csp-win32-x64', asset: 'csp-windows-x64.exe', binary: 'csp.exe', os: 'win32', cpu: 'x64' },
]

const [, , version, assetsDir] = process.argv
if (!version || !assetsDir) {
  process.stderr.write('usage: generate-platform-packages.mjs <version> <assets-dir>\n')
  process.exit(1)
}

const distRoot = join(npmRoot, 'dist')
mkdirSync(distRoot, { recursive: true })

const base = JSON.parse(readFileSync(join(npmRoot, 'csp', 'package.json'), 'utf8'))

for (const t of TARGETS) {
  const outDir = join(distRoot, t.pkg.replace('/', '__'))
  mkdirSync(outDir, { recursive: true })

  const pkg = {
    name: t.pkg,
    version,
    description: `csp binary for ${t.os}-${t.cpu}${t.libc ? ` (${t.libc})` : ''}.`,
    homepage: base.homepage,
    repository: base.repository,
    license: base.license,
    os: [t.os],
    cpu: [t.cpu],
    ...(t.libc ? { libc: [t.libc] } : {}),
    files: [t.binary],
  }
  writeFileSync(join(outDir, 'package.json'), `${JSON.stringify(pkg, null, 2)}\n`)

  const src = join(assetsDir, t.asset)
  const dest = join(outDir, t.binary)
  copyFileSync(src, dest)
  chmodSync(dest, 0o755)
  process.stdout.write(`wrote ${t.pkg}@${version} (${t.asset} -> ${t.binary})\n`)
}

// Stamp the wrapper with the release version + pinned optionalDependencies.
const wrapper = {
  ...base,
  version,
  optionalDependencies: Object.fromEntries(TARGETS.map(t => [t.pkg, version])),
}
const wrapperDir = join(distRoot, 'csp')
mkdirSync(join(wrapperDir, 'bin'), { recursive: true })
writeFileSync(join(wrapperDir, 'package.json'), `${JSON.stringify(wrapper, null, 2)}\n`)
copyFileSync(join(npmRoot, 'csp', 'bin', 'csp.js'), join(wrapperDir, 'bin', 'csp.js'))
process.stdout.write(`wrote wrapper @pleaseai/csp@${version}\n`)
