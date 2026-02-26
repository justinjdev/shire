# Package Discovery

## MODIFIED Requirements

### Requirement: Source file scanning

After packages are indexed, source files within each package directory are scanned for symbol extraction.

#### Scenario: Source files discovered per ecosystem

- **WHEN** symbol extraction runs for a package
- **THEN** the package's directory is walked for source files matching the package's ecosystem
- **AND** file extensions are: `.ts`, `.tsx`, `.js`, `.jsx` (npm), `.go` (go), `.rs` (cargo), `.py` (python)

#### Scenario: Excluded directories

- **WHEN** walking a package directory for source files
- **THEN** directories named `node_modules`, `target`, `dist`, `.build`, `vendor`, `test`, `tests`, `__tests__`, `__pycache__` are skipped
