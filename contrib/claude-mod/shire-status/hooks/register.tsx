import { atom, read, update } from 'claude-code'
import type { EngineInterface, Register, Timer } from 'claude-code'

import type { ShireStatus } from '../types'
import { detailRows, statusLine, transitions, warnings } from './format'

const status = atom({ plugin: 'shire-status', key: 'status' } as const, null)
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
let polling = false
let soon: Timer | undefined

async function poll($: EngineInterface): Promise<void> {
  if (polling) return
  polling = true
  try {
    const ran = await $.process.run(['shire', 'status', '--json'], { timeoutMs: 10_000 })
    if (ran.exitCode !== 0) {
      // An older shire has no `status` subcommand: clap exits 2.
      const why = ran.stderr.includes('unrecognized subcommand')
        ? 'this shire has no `status` command; upgrade shire'
        : ran.stderr.trim().split('\n')[0] || `shire status exited ${ran.exitCode}`
      await fail($, why)
      return
    }
    const next = JSON.parse(ran.stdout) as ShireStatus
    const prev = await read($, status)
    for (const line of transitions(prev, next)) $.ui.toast(`shire: ${line}`)
    await update($, status, () => next)
    await update($, error, () => null)
    $.ui.status(statusLine(next, await $.clock.now()))
  } catch (e) {
    // `$.process.run` rejects when the command cannot start at all.
    await fail($, `shire not found on PATH (${e instanceof Error ? e.message : String(e)})`)
  } finally {
    polling = false
  }
}

async function fail($: EngineInterface, why: string): Promise<void> {
  const had = await read($, error)
  await update($, error, () => why)
  await update($, status, () => null)
  $.ui.status('shire ✗ unavailable (/shire for details)')
  if (had !== why) $.ui.toast(`shire: ${why}`)
}

/** `shire rebuild` when the watch daemon is up (it debounces and builds), else a build. */
async function rebuild($: EngineInterface, force: boolean): Promise<string> {
  const s = await read($, status)
  const root = s?.root
  const rootArgs = root ? ['--root', root] : []
  const viaDaemon = !force && s?.watch.running === true
  const argv = viaDaemon
    ? ['shire', 'rebuild', ...rootArgs]
    : ['shire', 'build', ...rootArgs, ...(force ? ['--force'] : [])]
  const label = force ? 'force rebuild' : viaDaemon ? 'rebuild (watch daemon)' : 'rebuild'
  if ((await read($, busy)) !== null) return 'shire: a rebuild is already running'
  await update($, busy, () => label)
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
    await update($, busy, () => null)
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
    const running = await read($, busy)
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
