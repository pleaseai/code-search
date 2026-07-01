#!/usr/bin/env node
// Fallback launcher for the platform-specific `csp` Rust binary.
//
// Normally the postinstall step (`install.js`) copies the native binary OVER
// this file so npm's `.bin/csp` symlink resolves straight to native code — no
// Node.js process on the hot path (esbuild/ast-grep-style copy-over). This JS
// shim is the fallback for when postinstall did not run — e.g. `--ignore-scripts`,
// or bun blocking lifecycle scripts for untrusted deps (`bunx @pleaseai/csp`,
// or `bun install` without `@pleaseai/csp` in `trustedDependencies`). It resolves
// the binary at runtime and execs it, forwarding argv, stdio, signals, and the
// exit code.

const { spawn } = require('node:child_process')
const process = require('node:process')
const { resolveBinaryPath, resolveDevBinaryPath, resolvePlatformPackage } = require('../lib/resolve.js')

function main() {
  const target = resolvePlatformPackage()
  if (target === null) {
    process.stderr.write(
      `csp: unsupported platform ${process.platform}/${process.arch}.\n`
      + 'See https://github.com/pleaseai/code-search/releases for prebuilt binaries.\n',
    )
    process.exit(1)
  }

  const binaryPath = resolveBinaryPath() ?? resolveDevBinaryPath()
  if (binaryPath === null) {
    process.stderr.write(
      `csp: the platform package "${target.pkg}" is not installed.\n`
      + 'It should have been pulled in automatically as an optional dependency. '
      + 'Try reinstalling without --no-optional, or download a binary from '
      + 'https://github.com/pleaseai/code-search/releases.\n',
    )
    process.exit(1)
  }

  // This shim only runs when the postinstall copy-over did not (on Windows the
  // shim is the intended launcher, so no hint there). Nudge interactive users
  // toward the fast path; stay silent on pipes/MCP-stdio/CI to avoid noise.
  if (process.platform !== 'win32' && process.stderr.isTTY && !process.env.CSP_NO_FALLBACK_WARNING) {
    process.stderr.write(
      'csp: running via the Node launcher (postinstall copy-over did not run), '
      + 'which adds per-invocation startup overhead.\n'
      + 'Reinstall without --ignore-scripts, or under bun add "@pleaseai/csp" to '
      + '"trustedDependencies", for the native fast path. '
      + 'Set CSP_NO_FALLBACK_WARNING=1 to silence this.\n',
    )
  }

  const child = spawn(binaryPath, process.argv.slice(2), {
    stdio: 'inherit',
    windowsHide: true,
  })

  child.on('error', (error) => {
    process.stderr.write(`csp: failed to execute native binary: ${error.message}\n`)
    process.exit(1)
  })

  // Forward termination signals to the child so a supervisor killing this
  // launcher (e.g. stopping a long-running `csp mcp` server) cleanly stops the
  // binary too, rather than orphaning it.
  const signals = ['SIGINT', 'SIGTERM', 'SIGHUP']
  const forward = signals.map((signal) => {
    const handler = () => {
      if (!child.killed) {
        try {
          child.kill(signal)
        }
        catch {}
      }
    }
    process.on(signal, handler)
    return [signal, handler]
  })

  child.on('exit', (code, signal) => {
    for (const [s, h] of forward) {
      process.removeListener(s, h)
    }
    if (signal) {
      // Re-raise the signal on ourselves so the parent observes the same cause
      // of death (correct exit status for shells and supervisors).
      process.kill(process.pid, signal)
    }
    else {
      process.exit(code ?? 1)
    }
  })
}

main()
