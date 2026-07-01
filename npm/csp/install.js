#!/usr/bin/env node
// Postinstall copy-over optimization (esbuild-style).
//
// Replace the JS launcher shim at `bin/csp.js` with the actual platform binary
// so npm's `.bin/csp` symlink resolves directly to native code — no Node.js
// process spawned on every invocation. If anything fails (unsupported platform,
// missing optional dependency, read-only filesystem, or a package manager that
// skips lifecycle scripts entirely), the JS shim stays in place and still works
// as a runtime fallback. This step is therefore best-effort and MUST exit 0.
//
// It is also idempotent: it only requires `lib/resolve.js` (never the shim it
// overwrites), so a repeated `npm rebuild` re-copies the binary cleanly instead
// of trying to `require()` a native executable as CommonJS.

const { chmodSync, copyFileSync, linkSync, renameSync, statSync, unlinkSync } = require('node:fs')
const { join, sep } = require('node:path')
const process = require('node:process')
const { resolveBinaryPath } = require('./lib/resolve.js')

function main() {
  // Only rewrite the launcher when running as an installed dependency. From a
  // source checkout `bin/csp.js` is a git-tracked file, and an in-place
  // copy-over there would mutate the repo — the generator reads that same file
  // into every published wrapper, so one stray local install could ship a
  // native binary as the "launcher". An installed package always lives under a
  // `node_modules/` path segment; a checkout does not.
  if (!__dirname.split(sep).includes('node_modules')) {
    return
  }

  // On Windows the npm-generated bin shims (csp.cmd / csp.ps1) invoke
  // `node bin/csp.js`, so the shim must remain JavaScript. Skip the copy-over
  // and rely on the runtime launcher there.
  if (process.platform === 'win32') {
    return
  }

  const binaryPath = resolveBinaryPath()
  if (binaryPath === null) {
    // Unsupported platform or the optional dependency was not installed; the JS
    // shim already prints a helpful message at runtime.
    return
  }

  const shimPath = join(__dirname, 'bin', 'csp.js')

  // Idempotency short-circuit: if the shim was already replaced by a hard link
  // to the binary, there is nothing to do. This matters because POSIX rename()
  // between two hard links to the same inode is a no-op that leaves the temp
  // file behind — so on a re-run (`npm rebuild`, `npm ci`) we must not enter the
  // link+rename path at all.
  try {
    const shimStat = statSync(shimPath)
    const binStat = statSync(binaryPath)
    // Match on device + inode: inode numbers are only unique within a
    // filesystem, so comparing ino alone could collide across devices.
    if (shimStat.dev === binStat.dev && shimStat.ino === binStat.ino) {
      return
    }
  }
  catch {}

  const tempPath = `${shimPath}.tmp-${process.pid}`

  try {
    // Prefer a hard link (instant, no byte copy, shares the binary's inode);
    // fall back to a real copy across filesystems. The platform package already
    // ships the binary mode 0755, so only chmod on the copy path (a hard link
    // shares the source inode — mutating its mode could touch a shared store).
    // Write to a temp path then atomically rename over the shim so a concurrent
    // exec never observes a half-written file.
    try {
      linkSync(binaryPath, tempPath)
    }
    catch {
      copyFileSync(binaryPath, tempPath)
      chmodSync(tempPath, 0o755)
    }
    renameSync(tempPath, shimPath)
  }
  catch {
    // Leave the JS shim in place as the fallback.
  }
  finally {
    // Best-effort cleanup: rename() normally consumes the temp file (ENOENT
    // here, ignored); this removes any residue if it did not.
    try {
      unlinkSync(tempPath)
    }
    catch {}
  }
}

main()
