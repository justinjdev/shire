/** One `shire status --json` object. */
export type ShireStatus = {
  shire_version: string
  root: string
  /** null when shire.toml could not be read (state is then `unreadable`). */
  db_path: string | null
  state: 'missing' | 'refused' | 'unreadable' | 'building' | 'interrupted' | 'ok'
  error: string | null
  db_size_bytes: number | null
  build_running: boolean
  indexed_at: string | null
  build_duration_ms: number | null
  git_commit: string | null
  head_commit: string | null
  head_matches: boolean | null
  counts: {
    packages: number | null
    symbols: number | null
    references: number | null
    files: number | null
    docs: number | null
  }
  references_enabled: boolean | null
  file_walk: 'complete' | 'partial' | 'capped' | null
  pending_source_recheck: string[]
  last_build_failures: { kind: string; target: string; error: string }[]
  watch: { running: boolean; listening: boolean; pid: number | null }
}

declare module 'claude-code' {
  interface PluginState {
    'shire-status': {
      /** The last status read; null before the first poll or when it failed. */
      status: ShireStatus | null
      /** Why the last poll failed (shire not installed, an old shire, ...). */
      error: string | null
      /** The last snapshot taken while no build ran; toasts compare against it. */
      baseline: ShireStatus | null
      /**
       * What a pane button is running right now, if anything. `owner` names
       * the module load that started it: state survives a hot reload but the
       * rebuild's `finally` may not, so another load's entry is stale.
       */
      busy: { label: string; owner: string } | null
    }
  }
}
