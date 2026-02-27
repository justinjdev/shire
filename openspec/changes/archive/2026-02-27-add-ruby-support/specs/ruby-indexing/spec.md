# Ruby Indexing

## ADDED Requirements

### Requirement: Gemfile package discovery

#### Scenario: Simple Gemfile

- **WHEN** a directory contains a `Gemfile` with `gem 'rails', '~> 7.0'`
- **THEN** a package SHALL be created with:
  - name: directory path relative to repo root
  - kind: `ruby`
  - dependency: `rails` with version_req `~> 7.0` and dep_kind `Runtime`

#### Scenario: Gemfile with unversioned dependency

- **WHEN** a `Gemfile` contains `gem 'pg'`
- **THEN** a dependency SHALL be created with name `pg`, no version_req, and dep_kind `Runtime`

#### Scenario: Gemfile with test dependencies

- **WHEN** a `Gemfile` contains `group :test do gem 'rspec', '~> 3.0' end`
- **THEN** a dependency SHALL be created with name `rspec`, version_req `~> 3.0`, and dep_kind `Dev`

#### Scenario: Gemfile with development dependencies

- **WHEN** a `Gemfile` contains `group :development do gem 'pry' end`
- **THEN** a dependency SHALL be created with name `pry`, no version_req, and dep_kind `Dev`

#### Scenario: Gemfile with dynamic Ruby code

- **WHEN** a `Gemfile` contains Ruby expressions beyond simple `gem` statements
- **THEN** unparseable lines SHALL be silently skipped
- **AND** successfully parsed dependencies SHALL still be extracted

### Requirement: Ruby symbol extraction

#### Scenario: Class definition

- **WHEN** a `.rb` file contains `class PaymentService ... end`
- **THEN** a symbol SHALL be extracted with:
  - name: `PaymentService`
  - kind: `class`
  - visibility: `public`

#### Scenario: Class with inheritance

- **WHEN** a `.rb` file contains `class PaymentService < BaseService`
- **THEN** a symbol SHALL be extracted with kind `class` and signature including the superclass

#### Scenario: Module definition

- **WHEN** a `.rb` file contains `module Payments ... end`
- **THEN** a symbol SHALL be extracted with:
  - name: `Payments`
  - kind: `class`

#### Scenario: Instance method

- **WHEN** a `.rb` file contains `def process(amount, currency)` inside `class PaymentService`
- **THEN** a symbol SHALL be extracted with:
  - name: `process`
  - kind: `method`
  - parent_symbol: `PaymentService`
  - parameters: `[{"name": "amount"}, {"name": "currency"}]`

#### Scenario: Class method

- **WHEN** a `.rb` file contains `def self.create(attrs)` inside a class
- **THEN** a symbol SHALL be extracted with kind `function` and parent_symbol set to the class name

#### Scenario: Top-level method

- **WHEN** a `.rb` file contains `def helper_method` outside any class or module
- **THEN** a symbol SHALL be extracted with kind `function`

#### Scenario: Ruby script files

- **WHEN** a `.rb` file contains class, module, and method definitions
- **THEN** symbols SHALL be extracted using the same rules regardless of file location
