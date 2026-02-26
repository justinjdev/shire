# Build Timing

## ADDED Requirements

### Requirement: Per-phase timing measurement

Each phase of the build pipeline SHALL be individually timed using `std::time::Instant`. Timing MUST cover all 9 phases of `build_index`.

#### Scenario: All phases are timed

- **WHEN** `shire build` runs against a repository
- **THEN** the following phases are each individually timed:
  1. walk — manifest discovery
  2. workspace-context — Cargo workspace deps and Go workspace dir collection
  3. diff — hash comparison against stored hashes
  4. parse — manifest parsing for new and changed manifests
  5. remove-deleted — cleanup of packages from removed manifests
  6. recompute-internals — recompute `is_internal` for all dependencies
  7. update-hashes — persist content hashes for parsed manifests
  8. extract-symbols — symbol extraction and source hashing for new/changed packages, plus source-level re-extraction for unchanged packages with modified sources
  9. index-files — file walking, package association, and insertion
- **AND** each phase's duration is captured as a `std::time::Duration`

### Requirement: Timing output to stderr

WHEN a build completes, a timing breakdown MUST be printed to stderr showing every phase name and its duration in milliseconds, followed by the total build duration.

#### Scenario: Timing breakdown format

- **WHEN** `shire build` completes successfully
- **THEN** the following block is printed to stderr:
  ```
  Build timing:
    walk             <N>ms
    workspace-context <N>ms
    diff              <N>ms
    parse             <N>ms
    remove-deleted    <N>ms
    recompute-internals <N>ms
    update-hashes     <N>ms
    extract-symbols   <N>ms
    index-files       <N>ms
    total             <N>ms
  ```
- **AND** each `<N>` is an integer representing milliseconds
- **AND** the output goes to stderr, NOT stdout

#### Scenario: Timing prints after build summary

- **WHEN** `shire build` completes
- **THEN** the timing breakdown is printed after the `Indexed N packages...` summary line on stdout
- **AND** the two outputs do not interfere (summary on stdout, timing on stderr)

### Requirement: Total duration stored in shire_meta

WHEN a build completes, the total wall-clock duration MUST be stored in the `shire_meta` table.

#### Scenario: total_duration_ms key

- **WHEN** `shire build` completes
- **THEN** a row with key `total_duration_ms` EXISTS in `shire_meta`
- **AND** the value is a string representation of the total build duration in milliseconds (e.g., `"183"`)

#### Scenario: Querying total duration

- **WHEN** a user runs `SELECT value FROM shire_meta WHERE key = 'total_duration_ms'`
- **THEN** the result is the millisecond duration of the most recent build
- **AND** the value is overwritten on each subsequent build

### Requirement: Timing with --force flag

Timing instrumentation SHALL work identically regardless of whether `--force` is passed.

#### Scenario: Force build timing

- **WHEN** `shire build --force` runs
- **THEN** all 9 phases are timed
- **AND** the timing breakdown is printed to stderr
- **AND** `total_duration_ms` is stored in `shire_meta`
- **AND** the timing output format is identical to a non-force build
