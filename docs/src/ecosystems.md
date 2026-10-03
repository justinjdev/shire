# Supported Ecosystems

| Manifest | Kind | Workspace support |
|---|---|---|
| `package.json` | npm | `workspace:` protocol versions normalized |
| `go.mod` | go | `go.work` member metadata |
| `go.work` | go | `use` directives parsed for workspace context |
| `Cargo.toml` | cargo | `workspace = true` deps resolved from root |
| `pyproject.toml` | python | — |
| `pom.xml` | maven | Parent POM inheritance (groupId, version) |
| `build.gradle` / `build.gradle.kts` | gradle | `settings.gradle` project inclusion |
| `cpanfile` | perl | `requires` / `on 'test'` blocks |
| `Gemfile` | ruby | `gem` / `group :test` blocks |
| `flake.nix` | nix | `inputs` attrset (dotted and block forms) |

## Package naming

A package's name is the join key everything else carries (`symbols.package`,
`dependencies.package`, the `package` filter on every MCP tool), so it is
never empty:

| Manifest | Name |
|---|---|
| `package.json`, `pyproject.toml`, `Cargo.toml` | the declared name |
| `go.mod` | the last segment of the `module` path |
| `pom.xml`, `build.gradle` | `group:artifact` (`artifact` alone when there is no group) |
| `cpanfile`, `Gemfile`, `flake.nix` | no name field exists — see below |

When a manifest declares no name (a `Gemfile`, a tooling-only root
`pyproject.toml`, a private `package.json`), the name is derived from its
location: a nested manifest takes its directory path with `/` replaced by `-`
(`services/api/Gemfile` → `services-api`), and a manifest at the repo root
takes the repository directory's own name.

Two Gradle subprojects can compute the same `group:projectName` (two
directories both called `app`). The one indexed first keeps that name; the
colliding one falls back to its path-derived name, with `-2`, `-3`… appended
if that name is taken as well. A warning naming both directories is logged,
and neither package is dropped.

## Symbol extraction

Shire extracts symbols (functions, classes, types, methods, interfaces) from source files using [tree-sitter](https://tree-sitter.github.io/tree-sitter/), with full signatures, parameters, and return types.

Private and unexported symbols are indexed too, not skipped. Every symbol
carries a `visibility` — `public`, `protected`, `internal` or `private` —
derived from the language's own convention (the Visibility column below). A
member is narrowed by its enclosing type: a public method of a private class
is `private`. Search ranks private symbols after the others (see
[MCP Tools](mcp-tools.md)), and `symbols.include_private = false` drops them
(see [Configuration](configuration.md)) — only `private` ones: `internal`
(e.g. Java package-private, Rust `pub(crate)`) and `protected` symbols are
always kept. Where a language has no visibility rule that Shire reads, every
symbol is `public`.

| Language | Extractor | Visibility |
|---|---|---|
| TypeScript / JavaScript | tree-sitter | Module-level declarations: exported (`export ...` or named in an `export { ... }` clause) is `public`, anything else `private`. Methods: `private` / `#name` / `protected` modifiers. CommonJS `module.exports` is not recognised. |
| Go | tree-sitter | Capitalised name `public`, otherwise `private` |
| Rust | tree-sitter | `pub` → `public`; `pub(crate)` / `pub(super)` / `pub(in …)` → `internal`; no modifier or `pub(self)` → `private`. Trait-impl methods are `public`. |
| Python | tree-sitter | Leading `_` → `private`; dunder names (`__init__`) are `public` |
| Java | tree-sitter | `public` / `protected` / `private`; package-private (no modifier) → `internal`. Interface members and enum constants are implicitly `public`. |
| Kotlin | tree-sitter | `private` / `protected` / `internal`; no modifier → `public` |
| Dart | tree-sitter | Leading `_` → `private` (including named constructors such as `Foo._internal`) |
| Protobuf | tree-sitter | all `public` |
| C | tree-sitter | `static` → `private`, otherwise `public` |
| C++ | tree-sitter | Class members from the nearest `public:` / `protected:` / `private:` label (default `private` in a `class`, `public` in a `struct`); non-member `static` → `private` |
| C# | tree-sitter | `public` / `protected` / `internal` / `private`; with no modifier a class member is `private`, an interface member `public`, a top-level type `internal` |
| Swift | tree-sitter | `private` / `fileprivate` → `private`; explicit `internal` → `internal`; no modifier → `public` |
| PHP | tree-sitter | `private` / `protected`; no modifier → `public` |
| Scala | tree-sitter | `private` / `private[this]` → `private`; `private[pkg]` → `internal`; `protected` |
| Zig | tree-sitter | `pub` → `public`, otherwise `private` |
| Bash / Shell | tree-sitter | all `public` |
| R | tree-sitter | all `public` |
| Haskell | tree-sitter | all `public` (export lists are not read) |
| YAML | tree-sitter | all `public` |
| SQL | tree-sitter | all `public` |
| HCL / Terraform | tree-sitter | all `public` |
| TOML | tree-sitter | all `public` |
| Perl | tree-sitter | Leading `_` → `private` |
| Ruby | tree-sitter | Methods after a bare `private` / `protected` line, or written `private def …`; `private :name` is not tracked |
| OCaml | tree-sitter | all `public` (`.mli` signatures are not read) |
| Lua | tree-sitter | `local function` / `local f = function` → `private` |
| Elixir | tree-sitter | `defp` / `defmacrop` / `defguardp` / `@typep` → `private` |
| Clojure | tree-sitter | `defn-` and `^:private` metadata → `private` |
| Erlang | tree-sitter | all `public` (`-export` lists are not read) |
| Julia | tree-sitter | all `public` (`export` statements are not read) |
| Gleam | tree-sitter | `pub` → `public`, otherwise `private` |
| Odin | tree-sitter | all `public` |
| Nix | tree-sitter | all `public` |
| Nim | tree-sitter | `*` export marker → `public`, otherwise `private` |
| COBOL | regex-based | all `public` |

An index built by an older Shire picks the private symbols up on its first
build after upgrading: the extractor version is stored in the index, and a
mismatch re-extracts every source file once.

## Reference extraction

Shire extracts cross-references (calls, type references, imports, and interface implementations) for a subset of languages. These are stored in the `symbol_refs` table and exposed via the `symbol_references`, `symbol_callers`, and `symbol_callees` MCP tools.

| Language | Call | Type | Import | Impl |
|---|---|---|---|---|
| Go | yes | yes | yes | — (implicit interfaces) |
| Python | yes | yes | yes | yes |
| Java | yes | yes | yes | yes |
| TypeScript | yes | yes | yes | yes |
| JavaScript | yes | — | yes | yes |
| Perl | yes | — | yes | — |
| Ruby | yes | yes | yes | yes |
| Scala | yes | yes | yes | yes |

All other languages: symbol definitions only; references are not extracted.
