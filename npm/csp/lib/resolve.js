// Shared platform-binary resolution for the `csp` npm wrapper.
//
// This module is required by BOTH the runtime launcher (`bin/csp.js`) and the
// postinstall copy-over step (`install.js`). It must NEVER be the file that the
// copy-over overwrites — `bin/csp.js` is overwritten with the native binary at
// install time, so the resolution logic lives here where it stays JavaScript
// and can be required idempotently (`npm rebuild` runs postinstall again).

const { existsSync } = require('node:fs')
const { join } = require('node:path')
const process = require('node:process')

/**
 * Map the current platform/arch (plus libc on Linux) to the optional-dependency
 * package name and the binary filename it ships.
 *
 * @returns {{ pkg: string, binary: string } | null} the package/binary pair, or
 *   null when the current platform/arch is unsupported.
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
      // Only a glibc arm64 build exists. On musl arm64 (Alpine) return null so
      // the launcher reports an unsupported platform rather than exec'ing an
      // incompatible glibc binary.
      return musl ? null : { pkg: '@pleaseai/csp-linux-arm64', binary: 'csp' }
    }
  }

  return null
}

/** Best-effort libc detection: report.glibcVersionRuntime is absent on musl. */
function isMusl() {
  try {
    if (typeof process.report?.getReport !== 'function') {
      return false
    }
    // getReport() defaults to including network info, which can trigger a
    // blocking reverse-DNS (PTR) lookup — the opposite of this shim's goal.
    // We only need report.header.glibcVersionRuntime, so exclude network data
    // for the call and restore the caller's setting afterward.
    const previousExcludeNetwork = process.report.excludeNetwork
    process.report.excludeNetwork = true
    try {
      const report = process.report.getReport()
      if (report && report.header && report.header.glibcVersionRuntime) {
        return false
      }
      // No glibc runtime reported → assume musl (e.g. Alpine).
      return report != null
    }
    finally {
      process.report.excludeNetwork = previousExcludeNetwork
    }
  }
  catch {
    return false
  }
}

/**
 * Resolve the absolute path to the platform binary shipped by the matching
 * optional-dependency package, or `null` if the platform is unsupported or the
 * package is not installed.
 *
 * @returns {string | null} the absolute binary path, or null when unsupported
 *   or the platform package is not installed.
 */
function resolveBinaryPath() {
  const target = resolvePlatformPackage()
  if (target === null) {
    return null
  }
  try {
    return require.resolve(`${target.pkg}/${target.binary}`)
  }
  catch {
    return null
  }
}

/**
 * Locate a binary built into the repo's `target/` dir, for running the shim
 * straight from a source checkout (no published platform package installed).
 *
 * This is intentionally NOT consulted by the postinstall copy-over — it must
 * never copy a dev binary over the source `bin/csp.js`. Only the runtime
 * fallback launcher uses it, as a last resort after {@link resolveBinaryPath}.
 *
 * @returns {string | null} the absolute path to a locally built binary, or null.
 */
function resolveDevBinaryPath() {
  const target = resolvePlatformPackage()
  if (target === null) {
    return null
  }
  // lib/resolve.js → npm/csp/lib → npm/csp → npm → <repo root>
  for (const profile of ['release', 'debug']) {
    const dev = join(__dirname, '..', '..', '..', 'target', profile, target.binary)
    if (existsSync(dev)) {
      return dev
    }
  }
  return null
}

// isMusl stays private — it is an internal helper of resolvePlatformPackage;
// exporting it would invite callers to drift from the canonical resolution.
module.exports = { resolvePlatformPackage, resolveBinaryPath, resolveDevBinaryPath }
