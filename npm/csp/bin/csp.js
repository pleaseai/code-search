#!/usr/bin/env node
// Launcher for the platform-specific `csp` Rust binary. Resolves the binary
// shipped by the matching @pleaseai/csp-<platform> optional dependency and
// execs it, forwarding argv, stdio, and the exit code. Modeled on Biome's
// distribution launcher (ADR-0003 / T023).

const { spawnSync } = require('node:child_process')
const process = require('node:process')

/**
 * Map the current platform/arch (plus libc on Linux) to the optional-dependency
 * package name and the binary filename it ships.
 */
function resolvePlatformPackage() {
  const { platform, arch } = process

  if (platform === 'win32') {
    if (arch === 'x64') {
      return { pkg: '@pleaseai/csp-win32-x64', binary: 'csp.exe' }
    }
  }
  else if (platform === 'darwin') {
    if (arch === 'arm64') {
      return { pkg: '@pleaseai/csp-darwin-arm64', binary: 'csp' }
    }
    if (arch === 'x64') {
      return { pkg: '@pleaseai/csp-darwin-x64', binary: 'csp' }
    }
  }
  else if (platform === 'linux') {
    const musl = isMusl()
    if (arch === 'x64') {
      return musl
        ? { pkg: '@pleaseai/csp-linux-x64-musl', binary: 'csp' }
        : { pkg: '@pleaseai/csp-linux-x64', binary: 'csp' }
    }
    if (arch === 'arm64') {
      // arm64 ships glibc only for now; musl arm64 falls back to it.
      return { pkg: '@pleaseai/csp-linux-arm64', binary: 'csp' }
    }
  }

  return null
}

/** Best-effort libc detection: report.glibcVersionRuntime is absent on musl. */
function isMusl() {
  try {
    const report = typeof process.report?.getReport === 'function'
      ? process.report.getReport()
      : null
    if (report && report.header && report.header.glibcVersionRuntime) {
      return false
    }
    // No glibc runtime reported → assume musl (e.g. Alpine).
    return report !== null
  }
  catch {
    return false
  }
}

function main() {
  const target = resolvePlatformPackage()
  if (target === null) {
    process.stderr.write(
      `csp: unsupported platform ${process.platform}/${process.arch}.\n`
      + 'See https://github.com/pleaseai/code-search/releases for prebuilt binaries.\n',
    )
    process.exit(1)
  }

  let binaryPath
  try {
    binaryPath = require.resolve(`${target.pkg}/${target.binary}`)
  }
  catch {
    process.stderr.write(
      `csp: the platform package "${target.pkg}" is not installed.\n`
      + 'It should have been pulled in automatically as an optional dependency. '
      + 'Try reinstalling without --no-optional, or download a binary from '
      + 'https://github.com/pleaseai/code-search/releases.\n',
    )
    process.exit(1)
  }

  const result = spawnSync(binaryPath, process.argv.slice(2), {
    stdio: 'inherit',
    windowsHide: true,
  })
  if (result.error) {
    throw result.error
  }
  process.exit(result.status ?? 1)
}

main()
