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
| Elixir | regex-based |
| Perl | regex-based |
| Ruby | regex-based |
