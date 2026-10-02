import { expect, mock, test } from 'claude-code/testing'

import type { ShireStatus } from '../types'
import { health, statusLine, transitions, warnings } from '../hooks/format'

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
  expect(statusLine(failed, NOW)).toBe('shire ▲ 412 pkgs · 38.2k syms · 10m ago · HEAD moved · 1 build failure')
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
  expect(toasts[0]).toContain('shire not found on PATH')
})

test('an unreadable config still renders', () => {
  const s = status({ state: 'unreadable', db_path: null, error: 'bad shire.toml' })
  expect(statusLine(s, NOW)).toBe('shire ✗ index unreadable')
})
