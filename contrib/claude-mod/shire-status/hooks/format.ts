import type { ShireStatus } from '../types'

export type Health = 'ok' | 'busy' | 'warn' | 'bad'

export function health(s: ShireStatus): Health {
  switch (s.state) {
    case 'missing':
    case 'refused':
    case 'unreadable':
      return 'bad'
    case 'building':
      return 'busy'
    case 'interrupted':
      return 'warn'
    case 'ok':
      return warnings(s).length > 0 ? 'warn' : 'ok'
  }
}

/** Short reasons the index is degraded, worst first. */
export function warnings(s: ShireStatus): string[] {
  const out: string[] = []
  if (s.state === 'interrupted') out.push('last build interrupted')
  const n = s.last_build_failures.length
  if (n > 0) out.push(`${n} build failure${n === 1 ? '' : 's'}`)
  if (s.file_walk === 'capped') out.push('file walk capped')
  else if (s.file_walk === 'partial') out.push('unreadable paths')
  if (s.pending_source_recheck.length > 0 && !s.build_running) {
    out.push(`${s.pending_source_recheck.length} pending re-check`)
  }
  if (s.head_matches === false) out.push('HEAD moved')
  return out
}

/** 38211 -> "38.2k"; null -> "?". */
export function compact(n: number | null): string {
  if (n === null) return '?'
  if (n < 1000) return String(n)
  // From 999_500 up, "k" would round to "1000k".
  if (n < 999_500) return `${trim(n / 1000)}k`
  return `${trim(n / 1_000_000)}M`
}

function trim(x: number): string {
  return x >= 100 ? String(Math.round(x)) : x.toFixed(1).replace(/\.0$/, '')
}

/** Age of an RFC 3339 timestamp at `nowMs`: "12s", "4m", "3h", "2d". */
export function age(iso: string | null, nowMs: number): string | null {
  if (iso === null) return null
  const t = Date.parse(iso)
  if (Number.isNaN(t)) return null
  const s = Math.max(0, Math.round((nowMs - t) / 1000))
  if (s < 60) return `${s}s`
  if (s < 3600) return `${Math.floor(s / 60)}m`
  if (s < 86400) return `${Math.floor(s / 3600)}h`
  return `${Math.floor(s / 86400)}d`
}

const GLYPH: Record<Health, string> = { ok: '●', busy: '⟳', warn: '▲', bad: '✗' }

/** The one line pinned under the prompt. */
export function statusLine(s: ShireStatus, nowMs: number): string {
  const h = health(s)
  const head = `shire ${GLYPH[h]}`
  if (s.state === 'missing') return `${head} no index (run shire build)`
  if (h === 'bad') return `${head} index ${s.state}`
  const parts = [`${compact(s.counts.packages)} pkgs`, `${compact(s.counts.symbols)} syms`]
  if (s.state === 'building') parts.unshift('building…')
  else {
    const a = age(s.indexed_at, nowMs)
    if (a !== null) parts.push(`${a} ago`)
  }
  const [worst] = warnings(s)
  if (worst !== undefined) parts.push(worst)
  return `${head} ${parts.join(' · ')}`
}

/**
 * Whether a snapshot can be compared against. One taken while a build runs
 * may hold the last build's metadata or none at all (the build locks readers
 * out), so toasts compare the last settled snapshot with the next settled
 * one and skip the polls in between: otherwise a build a poll lands in loses
 * its "index built" or "reindexed" toast.
 */
export function settled(s: ShireStatus): boolean {
  return s.state !== 'building'
}

/**
 * Toasts for what changed between two settled snapshots. Deliberately quiet:
 * an on-demand rebuild (`serve --root`) rewrites `indexed_at` every few
 * seconds while nothing changes, so a rebuild is only worth a toast when the
 * counts moved, and build failures only when one is new.
 */
export function transitions(prev: ShireStatus | null, next: ShireStatus): string[] {
  if (prev === null) return []
  const out: string[] = []
  if (prev.state === 'missing' && next.state === 'ok') {
    out.push(`index built: ${compact(next.counts.symbols)} symbols`)
  } else if (next.indexed_at !== prev.indexed_at && next.indexed_at !== null) {
    const ds = delta(prev.counts.symbols, next.counts.symbols)
    const df = delta(prev.counts.files, next.counts.files)
    if (ds !== 0 || df !== 0) {
      const took = next.build_duration_ms === null ? '' : ` in ${next.build_duration_ms}ms`
      out.push(`reindexed${took}: ${signed(ds)} symbols, ${signed(df)} files`)
    }
  }
  // A manifest that fails to parse fails again on every build, so compare
  // what failed, not when the build ran.
  const seen = new Set(prev.last_build_failures.map(failureKey))
  const fresh = next.last_build_failures.filter(f => !seen.has(failureKey(f)))
  const nf = next.last_build_failures.length
  if (fresh[0] !== undefined) {
    out.push(`build had ${nf} failure${nf === 1 ? '' : 's'}: ${fresh[0].target}`)
  }
  if (next.state === 'interrupted' && prev.state !== 'interrupted') {
    out.push('a build was interrupted; the next build repairs the index')
  }
  if (next.file_walk === 'capped' && prev.file_walk !== 'capped') {
    out.push('file walk hit the 500k cap: exclude directories in shire.toml')
  }
  if (prev.watch.running && !next.watch.running) out.push('watch daemon stopped')
  return out
}

function failureKey(f: ShireStatus['last_build_failures'][number]): string {
  return `${f.kind}\u0000${f.target}`
}

function delta(a: number | null, b: number | null): number {
  return a === null || b === null ? 0 : b - a
}

function signed(n: number): string {
  return n > 0 ? `+${n}` : String(n)
}

/** The rows the /shire pane shows, label then value. */
export function detailRows(s: ShireStatus, nowMs: number): [string, string][] {
  const rows: [string, string][] = [
    ['state', s.state + (s.error ? ` (${s.error})` : '')],
    ['root', s.root],
    ['index', (s.db_path ?? '?') + (s.db_size_bytes === null ? '' : ` · ${bytes(s.db_size_bytes)}`)],
  ]
  if (s.indexed_at !== null) {
    const took = s.build_duration_ms === null ? '' : ` · took ${s.build_duration_ms}ms`
    rows.push(['indexed', `${age(s.indexed_at, nowMs) ?? '?'} ago${took}`])
  }
  if (s.git_commit !== null) {
    const moved = s.head_matches === false ? ` (HEAD now ${short(s.head_commit)})` : ''
    rows.push(['commit', short(s.git_commit) + moved])
  }
  const c = s.counts
  rows.push([
    'counts',
    `${compact(c.packages)} packages · ${compact(c.symbols)} symbols · ` +
      `${compact(c.references)} refs · ${compact(c.files)} files · ${compact(c.docs)} docs`,
  ])
  if (s.references_enabled !== null) {
    rows.push(['references', s.references_enabled ? 'on' : 'off'])
  }
  if (s.file_walk !== null) rows.push(['file walk', s.file_walk])
  if (s.pending_source_recheck.length > 0) {
    rows.push(['pending', s.pending_source_recheck.join(', ')])
  }
  const w = s.watch
  rows.push([
    'watch',
    w.running ? `running${w.pid === null ? '' : ` (pid ${w.pid})`}${w.listening ? '' : ', not listening'}` : 'not running',
  ])
  return rows
}

function short(sha: string | null): string {
  return sha === null ? '?' : sha.slice(0, 8)
}

function bytes(n: number): string {
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KiB`
  return `${(n / 1024 / 1024).toFixed(1)} MiB`
}
