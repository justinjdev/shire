# Symbol Extraction

## MODIFIED Requirements

### Requirement: Source file discovery

Source files are discovered within each indexed package's directory.

#### Scenario: File extension filtering

- **WHEN** extracting symbols for a package
- **THEN** files with ALL registered extensions SHALL be processed, regardless of package kind:
  - `.ts`, `.tsx`, `.js`, `.jsx`
  - `.go`
  - `.rs`
  - `.py`
  - `.java`, `.kt`
  - `.proto`
  - `.pm`, `.pl`
  - `.rb`

#### Scenario: Excluded extensions

- **WHEN** `[symbols].exclude_extensions` is configured
- **THEN** files matching excluded extensions SHALL be skipped during symbol extraction for all packages

#### Scenario: Skip test and generated files

- **WHEN** a source file is inside a directory named `test`, `tests`, `__tests__`, `__pycache__`, `node_modules`, `target`, or `dist`
- **THEN** it is skipped

#### Scenario: Skip vendored/generated files

- **WHEN** a source file matches patterns like `*.generated.ts`, `*.pb.go`, `*_test.go` (Go test files), or `build.rs`
- **THEN** it is skipped
