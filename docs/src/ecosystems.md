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

Shire extracts public symbols (functions, classes, types, methods, interfaces) from source files using [tree-sitter](https://tree-sitter.github.io/tree-sitter/), with full signatures, parameters, and return types.

| Language | Extractor |
|---|---|
| TypeScript / JavaScript | tree-sitter |
| Go | tree-sitter |
| Rust | tree-sitter |
| Python | tree-sitter |
| Java | tree-sitter |
| Kotlin | tree-sitter |
| Dart | tree-sitter |
| Protobuf | tree-sitter |
| C | tree-sitter |
| C++ | tree-sitter |
| C# | tree-sitter |
| Swift | tree-sitter |
| PHP | tree-sitter |
| Scala | tree-sitter |
| Zig | tree-sitter |
| Bash / Shell | tree-sitter |
| R | tree-sitter |
| Haskell | tree-sitter |
| YAML | tree-sitter |
| SQL | tree-sitter |
| HCL / Terraform | tree-sitter |
| TOML | tree-sitter |
| Perl | tree-sitter |
| Ruby | tree-sitter |
| OCaml | tree-sitter |
| Lua | tree-sitter |
| Elixir | tree-sitter |
| Clojure | tree-sitter |
| Erlang | tree-sitter |
| Julia | tree-sitter |
| Gleam | tree-sitter |
| Odin | tree-sitter |
| Nix | tree-sitter |
| Nim | tree-sitter |
| COBOL | regex-based |

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
