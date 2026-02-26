# Symbol Extraction

## Requirements

### Requirement: Source file discovery

Source files are discovered within each indexed package's directory.

#### Scenario: File extension filtering

- **WHEN** extracting symbols for a package
- **THEN** only files with recognized extensions are processed:
  - npm: `.ts`, `.tsx`, `.js`, `.jsx`
  - go: `.go`
  - cargo: `.rs`
  - python: `.py`

#### Scenario: Skip test and generated files

- **WHEN** a source file is inside a directory named `test`, `tests`, `__tests__`, `__pycache__`, `node_modules`, `target`, or `dist`
- **THEN** it is skipped

#### Scenario: Skip vendored/generated files

- **WHEN** a source file matches patterns like `*.generated.ts`, `*.pb.go`, `*_test.go` (Go test files), or `build.rs`
- **THEN** it is skipped

### Requirement: TypeScript/JavaScript extraction

#### Scenario: Exported function

- **WHEN** a TS/JS file contains `export function processPayment(amount: number, currency: string): Promise<Receipt>`
- **THEN** a symbol is extracted with:
  - name: `processPayment`
  - kind: `function`
  - signature: `function processPayment(amount: number, currency: string): Promise<Receipt>`
  - parameters: `[{"name": "amount", "type": "number"}, {"name": "currency", "type": "string"}]`
  - return_type: `Promise<Receipt>`
  - visibility: `public`

#### Scenario: Exported class

- **WHEN** a TS/JS file contains `export class AuthService`
- **THEN** a symbol is extracted with kind `class`
- **AND** public methods within the class are extracted with kind `method` and `parent_symbol: "AuthService"`

#### Scenario: Exported interface

- **WHEN** a TS file contains `export interface UserConfig { ... }`
- **THEN** a symbol is extracted with kind `interface` and the interface signature

#### Scenario: Exported type alias

- **WHEN** a TS file contains `export type Result<T> = Success<T> | Failure`
- **THEN** a symbol is extracted with kind `type`

#### Scenario: Exported const/enum

- **WHEN** a TS/JS file contains `export const MAX_RETRIES = 3` or `export enum Status { ... }`
- **THEN** symbols are extracted with kind `constant` or `enum` respectively

#### Scenario: Default export

- **WHEN** a file contains `export default class Foo` or `export default function bar()`
- **THEN** the symbol is extracted with its name (Foo, bar)

#### Scenario: Non-exported symbols are skipped

- **WHEN** a function or class is not exported
- **THEN** it is not indexed

### Requirement: Go extraction

#### Scenario: Exported function

- **WHEN** a `.go` file contains `func ProcessPayment(amount float64, currency string) (*Receipt, error)`
- **THEN** a symbol is extracted with:
  - name: `ProcessPayment`
  - kind: `function`
  - signature: `func ProcessPayment(amount float64, currency string) (*Receipt, error)`
  - parameters: `[{"name": "amount", "type": "float64"}, {"name": "currency", "type": "string"}]`
  - return_type: `(*Receipt, error)`

#### Scenario: Exported type (struct)

- **WHEN** a `.go` file contains `type AuthService struct { ... }`
- **THEN** a symbol is extracted with kind `struct`

#### Scenario: Exported type (interface)

- **WHEN** a `.go` file contains `type Handler interface { ... }`
- **THEN** a symbol is extracted with kind `interface`

#### Scenario: Method with receiver

- **WHEN** a `.go` file contains `func (s *AuthService) Validate(token string) error`
- **THEN** a symbol is extracted with kind `method`, parent_symbol `AuthService`

#### Scenario: Unexported symbols are skipped

- **WHEN** a Go function or type starts with a lowercase letter
- **THEN** it is not indexed

### Requirement: Rust extraction

#### Scenario: Public function

- **WHEN** a `.rs` file contains `pub fn process_payment(amount: f64, currency: &str) -> Result<Receipt>`
- **THEN** a symbol is extracted with:
  - name: `process_payment`
  - kind: `function`
  - signature: `pub fn process_payment(amount: f64, currency: &str) -> Result<Receipt>`
  - parameters: `[{"name": "amount", "type": "f64"}, {"name": "currency", "type": "&str"}]`
  - return_type: `Result<Receipt>`

#### Scenario: Public struct

- **WHEN** a `.rs` file contains `pub struct AuthService { ... }`
- **THEN** a symbol is extracted with kind `struct`

#### Scenario: Public enum

- **WHEN** a `.rs` file contains `pub enum Status { Active, Inactive }`
- **THEN** a symbol is extracted with kind `enum`

#### Scenario: Public trait

- **WHEN** a `.rs` file contains `pub trait Handler { ... }`
- **THEN** a symbol is extracted with kind `trait`

#### Scenario: Impl method

- **WHEN** a `.rs` file contains `pub fn validate(&self, token: &str) -> Result<()>` inside `impl AuthService`
- **THEN** a symbol is extracted with kind `method`, parent_symbol `AuthService`

#### Scenario: Non-pub symbols are skipped

- **WHEN** a function, struct, or enum lacks the `pub` keyword
- **THEN** it is not indexed

### Requirement: Python extraction

#### Scenario: Top-level function

- **WHEN** a `.py` file contains `def process_payment(amount: float, currency: str) -> Receipt:`
- **THEN** a symbol is extracted with:
  - name: `process_payment`
  - kind: `function`
  - parameters: `[{"name": "amount", "type": "float"}, {"name": "currency", "type": "str"}]`
  - return_type: `Receipt`

#### Scenario: Class definition

- **WHEN** a `.py` file contains `class AuthService:`
- **THEN** a symbol is extracted with kind `class`
- **AND** methods not prefixed with `_` are extracted with kind `method` and parent_symbol `AuthService`

#### Scenario: Dunder/private methods are skipped

- **WHEN** a class method starts with `_` (e.g., `__init__`, `_internal_method`)
- **THEN** it is not indexed (except `__init__` which is extracted to capture constructor signature)

#### Scenario: All top-level functions/classes are extracted

- **WHEN** a Python function or class is defined at module level
- **THEN** it is extracted (Python has no export keyword — all top-level definitions are public API)

### Requirement: Extraction resilience

#### Scenario: Unparseable file

- **WHEN** tree-sitter fails to parse a source file (syntax errors, unsupported constructs)
- **THEN** the file is skipped silently
- **AND** extraction continues with remaining files

#### Scenario: Binary or non-text file

- **WHEN** a file with a recognized extension is actually binary
- **THEN** it is skipped without error
