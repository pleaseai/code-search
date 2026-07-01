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

import { chmodSync, copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const npmRoot = resolve(here, '..')

// asset = the file name emitted by release-rust.yml; binary = its name inside
// the published package (matches bin/csp.js resolution).
const TARGETS = [
  { pkg: '@pleaseai/csp-darwin-arm64', asset: 'csp-darwin-arm64', binary: 'csp', os: 'darwin', cpu: 'arm64' },
  { pkg: '@pleaseai/csp-darwin-x64', asset: 'csp-darwin-x64', binary: 'csp', os: 'darwin', cpu: 'x64' },
  { pkg: '@pleaseai/csp-linux-x64', asset: 'csp-linux-x64', binary: 'csp', os: 'linux', cpu: 'x64', libc: 'glibc' },
  { pkg: '@pleaseai/csp-linux-arm64', asset: 'csp-linux-arm64', binary: 'csp', os: 'linux', cpu: 'arm64', libc: 'glibc' },
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

// Repo root holds the README + LICENSE shipped inside the published packages.
const repoRoot = resolve(npmRoot, '..')

const base = JSON.parse(readFileSync(join(npmRoot, 'csp', 'package.json'), 'utf8'))

// Generate a package per target whose asset is present. A missing asset is
// skipped with a warning (so a partial matrix can still publish what built);
// only generated targets are pinned in the wrapper's optionalDependencies.
const generated = []
for (const t of TARGETS) {
  const src = join(assetsDir, t.asset)
  if (!existsSync(src)) {
    process.stderr.write(`skip ${t.pkg}: asset ${t.asset} not found in ${assetsDir}\n`)
    continue
  }

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

  const dest = join(outDir, t.binary)
  copyFileSync(src, dest)
  chmodSync(dest, 0o755)
  // Ship LICENSE in each platform package too — these are published
  // independently, and license-compliance scanners (FOSSA, Snyk, …) look for a
  // LICENSE file in every package directory.
  copyFileSync(join(repoRoot, 'LICENSE'), join(outDir, 'LICENSE'))
  generated.push(t)
  process.stdout.write(`wrote ${t.pkg}@${version} (${t.asset} -> ${t.binary})\n`)
}

if (generated.length === 0) {
  process.stderr.write('error: no assets matched any known target — nothing generated\n')
  process.exit(1)
}

// Stamp the wrapper with the release version + pinned optionalDependencies
// (only the targets actually generated this run).
const wrapper = {
  ...base,
  version,
  optionalDependencies: Object.fromEntries(generated.map(t => [t.pkg, version])),
}
const wrapperDir = join(distRoot, 'csp')
mkdirSync(join(wrapperDir, 'bin'), { recursive: true })
mkdirSync(join(wrapperDir, 'lib'), { recursive: true })
writeFileSync(join(wrapperDir, 'package.json'), `${JSON.stringify(wrapper, null, 2)}\n`)
// The runtime launcher shim (overwritten by the postinstall copy-over), the
// postinstall step itself, and the shared resolver both of them require.
copyFileSync(join(npmRoot, 'csp', 'bin', 'csp.js'), join(wrapperDir, 'bin', 'csp.js'))
copyFileSync(join(npmRoot, 'csp', 'install.js'), join(wrapperDir, 'install.js'))
copyFileSync(join(npmRoot, 'csp', 'lib', 'resolve.js'), join(wrapperDir, 'lib', 'resolve.js'))

// Ship the user-facing README + LICENSE in the published wrapper so the npm
// package page renders docs (without these, npm shows "No README data found").
// npm always includes README.md / LICENSE regardless of the `files` allowlist.
copyFileSync(join(repoRoot, 'README.md'), join(wrapperDir, 'README.md'))
copyFileSync(join(repoRoot, 'LICENSE'), join(wrapperDir, 'LICENSE'))
process.stdout.write(`wrote wrapper @pleaseai/csp@${version}\n`)
