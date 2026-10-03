import { expect, mock, test } from 'claude-code/testing'

import type { CommandRunInput } from 'claude-code'

import type { ShireStatus } from '../types'
import { compact, health, settled, statusLine, transitions, warnings } from '../hooks/format'

const NOW = Date.parse('2026-10-02T12:10:00Z')

function status(over: Partial<ShireStatus> = {}): ShireStatus {
  return {
    shire_version: '0.7.0',
    root: '/repo',
    db_path: '/repo/.shire/index.db',
    state: 'ok',
    error: null,
    db_size_bytes: 1_880_064,
    build_running: false,
    indexed_at: '2026-10-02T12:00:00Z',
    build_duration_ms: 180,
    git_commit: 'abc',
    head_commit: 'abc',
    head_matches: true,
    counts: { packages: 412, symbols: 38_211, references: 0, files: 261, docs: 12 },
    references_enabled: false,
    file_walk: 'complete',
    pending_source_recheck: [],
    last_build_failures: [],
    watch: { running: true, listening: true, pid: 42 },
    ...over,
  }
}

test('the status line summarises a healthy index', () => {
  expect(statusLine(status(), NOW)).toBe('shire ● 412 pkgs · 38.2k syms · 10m ago')
  expect(health(status())).toBe('ok')
})

test('the status line flags what needs attention', () => {
  expect(statusLine(status({ state: 'missing' }), NOW)).toBe('shire ✗ no index (run shire build)')
  expect(statusLine(status({ state: 'building', build_running: true }), NOW)).toBe(
    'shire ⟳ building… · 412 pkgs · 38.2k syms',
  )
  const failed = status({
    head_matches: false,
    last_build_failures: [{ kind: 'manifest', target: 'a/package.json', error: 'bad json' }],
  })
  expect(health(failed)).toBe('warn')
  expect(statusLine(failed, NOW)).toBe('shire ▲ 412 pkgs · 38.2k syms · 10m ago · 1 build failure')
})

test('HEAD moving alone is a warning', () => {
  const moved = status({ head_matches: false })
  expect(health(moved)).toBe('warn')
  expect(statusLine(moved, NOW)).toBe('shire ▲ 412 pkgs · 38.2k syms · 10m ago · HEAD moved')
})

test('counts just under a million round to 1M, not 1000k', () => {
  expect(compact(999_499)).toBe('999k')
  expect(compact(999_500)).toBe('1M')
  expect(compact(999_999)).toBe('1M')
  expect(compact(1_250_000)).toBe('1.3M')
})

test('pending re-checks only warn while no build is running', () => {
  expect(warnings(status({ pending_source_recheck: ['a'] }))).toEqual(['1 pending re-check'])
  expect(
    warnings(status({ state: 'building', build_running: true, pending_source_recheck: ['a'] })),
  ).toEqual([])
})

test('a rebuild that changed nothing makes no toast', () => {
  const later = status({ indexed_at: '2026-10-02T12:05:00Z' })
  expect(transitions(status(), later)).toEqual([])
  expect(transitions(null, later)).toEqual([])
})

test('changes worth knowing about make toasts', () => {
  const grew = status({
    indexed_at: '2026-10-02T12:05:00Z',
    counts: { packages: 412, symbols: 38_215, references: 0, files: 262, docs: 12 },
  })
  expect(transitions(status(), grew)).toEqual(['reindexed in 180ms: +4 symbols, +1 files'])
  expect(transitions(status(), status({ watch: { running: false, listening: false, pid: null } }))).toEqual([
    'watch daemon stopped',
  ])
  expect(transitions(status({ state: 'missing' }), status())).toEqual(['index built: 38.2k symbols'])
})

test('a failure that persists across builds toasts once', () => {
  const bad = { kind: 'manifest', target: 'fixtures/bad/package.json', error: 'bad json' }
  const first = status({ last_build_failures: [bad] })
  expect(transitions(status(), first)).toEqual(['build had 1 failure: fixtures/bad/package.json'])
  const again = status({ indexed_at: '2026-10-02T12:05:00Z', last_build_failures: [bad] })
  expect(transitions(first, again)).toEqual([])
  const other = { kind: 'extract', target: 'pkg-b', error: 'unreadable' }
  const more = status({ indexed_at: '2026-10-02T12:06:00Z', last_build_failures: [bad, other] })
  expect(transitions(again, more)).toEqual(['build had 2 failures: pkg-b'])
})

test('a build a poll lands in still gets its toast', () => {
  // A first build: the mid-build snapshot has no metadata yet.
  const building = status({
    state: 'building',
    build_running: true,
    indexed_at: null,
    counts: { packages: null, symbols: null, references: null, files: null, docs: null },
  })
  expect(settled(building)).toBe(false)
  // The mod compares settled snapshots, skipping the one taken mid-build.
  expect(transitions(status({ state: 'missing' }), status())).toEqual(['index built: 38.2k symbols'])
})

test('the mod polls shire and pins the status line', async ($, on) => {
  const clock = mock.clock(on, { now: NOW })
  const lines: (string | undefined)[] = []
  const argvs: string[][] = []
  on('session.start', () => ({ cwd: '/repo' }))
  on('command.register', (_$, e) => ({ value: { command: e.name } }))
  on('ui.status', (_$, e) => {
    lines.push(e.text)
    return { value: undefined }
  })
  on('ui.toast', () => ({ value: undefined }))
  on('process.run', (_$, e) => {
    argvs.push([...e.argv])
    return {
      value: {
        exitCode: 0,
        stdout: JSON.stringify(status()),
        stderr: '',
        isStdoutTruncated: false,
        isStderrTruncated: false,
      },
    }
  })

  await $.session.start({ cwd: '/repo', surface: 'terminal', isInteractive: true })
  await clock.settle()
  expect(argvs[0]).toEqual(['shire', 'status', '--json'])
  expect(lines.at(-1)).toBe('shire ● 412 pkgs · 38.2k syms · 10m ago')
})

test('a missing shire binary is reported, not thrown', async ($, on) => {
  const clock = mock.clock(on, { now: NOW })
  const lines: (string | undefined)[] = []
  const toasts: string[] = []
  on('session.start', () => ({ cwd: '/repo' }))
  on('command.register', (_$, e) => ({ value: { command: e.name } }))
  on('ui.status', (_$, e) => {
    lines.push(e.text)
    return { value: undefined }
  })
  on('ui.toast', (_$, e) => {
    toasts.push(e.text)
    return { value: undefined }
  })
  on('process.run', () => {
    throw new Error('spawn shire ENOENT')
  })

  await $.session.start({ cwd: '/repo', surface: 'terminal', isInteractive: true })
  await clock.settle()
  expect(lines.at(-1)).toBe('shire ✗ unavailable (/shire for details)')
  expect(toasts[0]).toContain('could not run shire status (is shire on PATH?)')
})

test('an unreadable config still renders', () => {
  const s = status({ state: 'unreadable', db_path: null, error: 'bad shire.toml' })
  expect(statusLine(s, NOW)).toBe('shire ✗ index unreadable')
})

test('output that is not JSON is reported as such, not as a missing shire', async ($, on) => {
  const clock = mock.clock(on, { now: NOW })
  const toasts: string[] = []
  on('session.start', () => ({ cwd: '/repo' }))
  on('command.register', (_$, e) => ({ value: { command: e.name } }))
  on('ui.status', () => ({ value: undefined }))
  on('ui.toast', (_$, e) => {
    toasts.push(e.text)
    return { value: undefined }
  })
  on('process.run', () => ({
    value: { exitCode: 0, stdout: 'warning: something\n', stderr: '', isStdoutTruncated: false, isStderrTruncated: false },
  }))

  await $.session.start({ cwd: '/repo', surface: 'terminal', isInteractive: true })
  await clock.settle()
  expect(toasts[0]).toBe('shire: shire status printed something that is not JSON: warning: something')
})

test('a rebuild goes through the daemon only when it is listening', async ($, on) => {
  const clock = mock.clock(on, { now: NOW })
  const argvs: string[][] = []
  let snapshot = status({ watch: { running: true, listening: false, pid: 42 } })
  on('session.start', () => ({ cwd: '/repo' }))
  on('command.register', (_$, e) => ({ value: { command: e.name } }))
  on('ui.status', () => ({ value: undefined }))
  on('ui.toast', () => ({ value: undefined }))
  on('ui.open', () => ({ value: { isPlaced: true as const } }))
  on('process.run', (_$, e) => {
    argvs.push([...e.argv])
    return {
      value: { exitCode: 0, stdout: JSON.stringify(snapshot), stderr: '', isStdoutTruncated: false, isStderrTruncated: false },
    }
  })

  await $.session.start({ cwd: '/repo', surface: 'terminal', isInteractive: true })
  await clock.settle()
  await $.command.run(run('rebuild'))
  expect(argvs).toContainEqual(['shire', 'build', '--root', '/repo'])

  snapshot = status()
  await $.command.run(run(''))
  argvs.length = 0
  await $.command.run(run('rebuild'))
  expect(argvs[0]).toEqual(['shire', 'rebuild', '--root', '/repo'])
})

/** A `/shire` run as the test drives it; the engine fills the rest. */
function run(args: string): CommandRunInput {
  return { command: 'shire', args } as CommandRunInput
}
