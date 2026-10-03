import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register, Timer } from 'claude-code'

import type { ShireStatus } from '../types'
import { detailRows, settled, statusLine, transitions, warnings } from './format'

const status = atom({ plugin: 'shire-status', key: 'status' } as const, null)
const baseline = atom({ plugin: 'shire-status', key: 'baseline' } as const, null)
const error = atom({ plugin: 'shire-status', key: 'error' } as const, null)
const busy = atom({ plugin: 'shire-status', key: 'busy' } as const, null)

const PANE = 'shire'
/** Background poll; `shire status` is read-only and takes a few ms. */
const POLL_MS = 15_000
/** Re-poll this long after an edit, so a watch-daemon rebuild shows up. */
const AFTER_EDIT_MS = 3_000
const EDIT_TOOLS = new Set(['Edit', 'Write', 'MultiEdit', 'NotebookEdit'])
/** A full build of a big monorepo runs for minutes. */
const BUILD_TIMEOUT_MS = 600_000

// Module variables: a reload starts them over, which is what they should do.
let soon: Timer | undefined
/** The poll running now, and the one queued behind it. */
let current: Promise<void> = Promise.resolve()
let queued: Promise<void> | null = null
/** Set synchronously, so two quick presses cannot both start a build. */
let rebuilding = false
/** Tells this module load's `busy` entry from one a reloaded module left. */
const OWNER = `${Date.now()}-${Math.random()}`

/**
 * Read `shire status`. Polls run one at a time, and a call made while one is
 * running waits for a fresh poll after it (shared by every such call), so a
 * caller never gets a snapshot taken before it asked.
 */
function poll($: EngineInterface): Promise<void> {
  if (queued !== null) return queued
  const run = current.then(() => {
    queued = null
    return pollOnce($)
  })
  queued = run
  current = run
  return run
}

async function pollOnce($: EngineInterface): Promise<void> {
  let ran
  try {
    ran = await $.process.run(['shire', 'status', '--json'], { timeoutMs: 10_000 })
  } catch (e) {
    // Rejects when the command cannot start (no shire on PATH) or times out;
    // the engine does not pass the cause through, so say both. A non-zero
    // exit or output that is not JSON are reported separately below.
    const msg = e instanceof Error ? e.message : String(e)
    await fail($, `could not run shire status (is shire on PATH?): ${msg}`)
    return
  }
  if (ran.exitCode !== 0) {
    // An older shire has no `status` subcommand: clap exits 2.
    const why = ran.stderr.includes('unrecognized subcommand')
      ? 'this shire has no `status` command; upgrade shire'
      : ran.stderr.trim().split('\n')[0] || `shire status exited ${ran.exitCode}`
    await fail($, why)
    return
  }
  let next: ShireStatus
  try {
    next = JSON.parse(ran.stdout) as ShireStatus
  } catch {
    const first = ran.stdout.trim().split('\n')[0]?.slice(0, 120) ?? ''
    await fail($, `shire status printed something that is not JSON: ${first || '(nothing)'}`)
    return
  }
  if (settled(next)) {
    for (const line of transitions(await read($, baseline), next)) $.ui.toast(`shire: ${line}`)
    await update($, baseline, () => next)
  }
  await update($, status, () => next)
  await update($, error, () => null)
  $.ui.status(statusLine(next, await $.clock.now()))
}

async function fail($: EngineInterface, why: string): Promise<void> {
  const had = await read($, error)
  await update($, error, () => why)
  await update($, status, () => null)
  $.ui.status('shire ✗ unavailable (/shire for details)')
  if (had !== why) $.ui.toast(`shire: ${why}`)
}

/**
 * `shire rebuild` when the watch daemon is up and listening (it debounces and
 * builds), else a build. Not when it is merely running: `shire rebuild`
 * exits 0 even when it cannot reach the socket, so a wedged daemon would make
 * the button report a rebuild that never happened.
 */
async function rebuild($: EngineInterface, force: boolean): Promise<string> {
  if (rebuilding) return 'shire: a rebuild is already running'
  rebuilding = true
  const s = await read($, status)
  const root = s?.root
  const rootArgs = root ? ['--root', root] : []
  const viaDaemon = !force && s?.watch.running === true && s.watch.listening
  const argv = viaDaemon
    ? ['shire', 'rebuild', ...rootArgs]
    : ['shire', 'build', ...rootArgs, ...(force ? ['--force'] : [])]
  const label = force ? 'force rebuild' : viaDaemon ? 'rebuild (watch daemon)' : 'rebuild'
  await update($, busy, () => ({ label, owner: OWNER }))
  $.ui.status(`shire ⟳ ${label}…`)
  try {
    const ran = await $.process.run(argv, { timeoutMs: BUILD_TIMEOUT_MS })
    const tail = (ran.stderr.trim().split('\n').pop() ?? '').slice(0, 200)
    return ran.exitCode === 0
      ? `shire: ${label} ${viaDaemon ? 'requested' : 'finished'}`
      : `shire: ${label} failed (exit ${ran.exitCode})${tail ? `: ${tail}` : ''}`
  } catch (e) {
    return `shire: ${label} could not run: ${e instanceof Error ? e.message : String(e)}`
  } finally {
    rebuilding = false
    await update($, busy, b => (b?.owner === OWNER ? null : b))
    await poll($)
  }
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'shire',
      description: 'shire index status; `/shire rebuild` or `/shire rebuild --force` to rebuild',
      argumentHint: '[rebuild [--force]]',
    })
    void poll($)
    $.clock.every(POLL_MS, () => void poll($))
    return next(e)
  })

  on('command.run', { command: 'shire' }, async ($, e) => {
    const args = e.args.trim().split(/\s+/).filter(Boolean)
    if (args[0] === 'rebuild') {
      return { text: await rebuild($, args.includes('--force')) }
    }
    await poll($)
    await $.ui.open({ id: PANE, title: 'shire' })
    const s = await read($, status)
    if (s === null) return { text: `shire: ${(await read($, error)) ?? 'no status yet'}` }
    const w = warnings(s)
    return {
      text: `shire: ${s.state}` + (w.length > 0 ? ` (${w.join(', ')})` : '') + ' — details in the pane.',
    }
  })

  // An edit may trigger a watch-daemon rebuild; look again shortly after.
  on('tool.call', async ($, e, next) => {
    const ran = await next(e)
    if (EDIT_TOOLS.has(String(e.tool))) {
      soon?.cancel()
      soon = $.clock.after(AFTER_EDIT_MS, () => void poll($))
    }
    return ran
  })

  on('ui.render', { component: 'Pane', requestId: PANE }, async ($, e) => {
    const { Box, Text, Button } = $.ui.resolve(e)
    const s = await read($, status)
    const err = await read($, error)
    const b = await read($, busy)
    const running = b?.owner === OWNER ? b.label : null
    const now = await $.clock.now()

    if (s === null) {
      return (
        <Box flexDirection="column">
          <Text color="red">{err ?? 'Reading shire status…'}</Text>
          <Text dimColor>Install or update shire, then run /shire again.</Text>
        </Box>
      )
    }

    const label = Math.max(...detailRows(s, now).map(([k]) => k.length)) + 2
    return (
      <Box flexDirection="column">
        {detailRows(s, now).map(([k, v]) => (
          <Box key={k}>
            <Text dimColor>{k.padEnd(label)}</Text>
            <Text>{v}</Text>
          </Box>
        ))}
        {s.last_build_failures.length > 0 && (
          <Box flexDirection="column" marginTop={1}>
            <Text color="yellow">Last build failures</Text>
            {s.last_build_failures.slice(0, 10).map((f, i) => (
              <Text key={`f${i}`} wrap="truncate-end">
                [{f.kind}] {f.target}: {f.error}
              </Text>
            ))}
          </Box>
        )}
        <Box marginTop={1}>
          {running !== null ? (
            <Text dimColor>{running}…</Text>
          ) : (
            <Box>
              <Button
                key="rebuild"
                label="Rebuild"
                hotkey="r"
                variant="primary"
                onPress={async () => $.ui.toast(await rebuild($, false))}
              />
              <Text> </Text>
              <Button
                key="force"
                label="Force rebuild"
                hotkey="f"
                onPress={async () => $.ui.toast(await rebuild($, true))}
              />
              <Text> </Text>
              <Button key="close" label="Close" role="dismiss" onPress={() => $.ui.close({ id: PANE })} />
            </Box>
          )}
        </Box>
      </Box>
    )
  })
}
