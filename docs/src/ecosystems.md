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
