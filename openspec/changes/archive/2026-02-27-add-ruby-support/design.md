## Context

Ruby has well-defined conventions: `Gemfile` for dependencies (parsed by Bundler) and `.rb` files with `class`, `module`, `def` declarations. The `Gemfile` is a Ruby DSL but common patterns are regex-parseable, similar to `cpanfile` and `build.gradle`.

## Goals / Non-Goals

**Goals:**
- Discover Ruby packages from `Gemfile` manifest files
- Extract symbols from `.rb` files (classes, modules, methods)
- Add `Gemfile` to the default manifest list

**Non-Goals:**
- `.gemspec` parsing (can be added later)
- Bundler version resolution or rubygems integration
- Rails-specific conventions (concerns, autoloading)
- Metaprogramming-generated methods (`define_method`, `method_missing`)

## Decisions

### 1. Gemfile as the manifest

`Gemfile` uses a Ruby DSL:
```ruby
gem 'rails', '~> 7.0'
gem 'pg'
group :test do
  gem 'rspec', '~> 3.0'
end
```

Parse `gem 'name'` lines for runtime deps, `group :test`/`group :development` blocks for dev deps. Same pragmatic regex approach as Gradle and cpanfile parsers.

### 2. Package naming

No explicit package name in `Gemfile`. Use directory path relative to repo root, consistent with Perl. Set `kind` to `"ruby"`.

### 3. Symbol extraction

| Ruby construct | SymbolKind | Notes |
|---|---|---|
| `class Foo` | `Class` | |
| `module Bar` | `Class` | Modules are Ruby's mixin/namespace mechanism, closest to class |
| `def method_name` (in class/module) | `Method` | With `parent_symbol` |
| `def method_name` (top-level) | `Function` | |
| `def self.class_method` | `Function` | Class-level method |

**Visibility:** Extract all methods not prefixed with `_`. Ruby uses `private`/`protected` keywords but they're method calls, not declarations — detecting them requires tracking state through the AST. For v1, extract all named methods.

### 4. tree-sitter-ruby

Use `tree-sitter-ruby`. Mature grammar, widely used in GitHub's own code navigation.

## Risks / Trade-offs

**[Gemfile complexity] Dynamic Gemfile contents** → Mitigation: Parse what we can, skip what we can't.

**[Ruby metaprogramming] Many Ruby methods are generated dynamically** → Mitigation: Static extraction will miss these. This is a known limitation shared by all static analysis tools for Ruby. The extractable surface is still valuable.
