# MCP Prompts

Prompts are pre-built templates for semantic codebase exploration. They compose multiple queries into structured context, giving your AI a map of where concepts live in the codebase.

| Prompt | Args | Description |
|---|---|---|
| `explore` | `query` | Search packages, symbols, and files for a concept — returns a structured context map organized by package |
| `explore-package` | `name` | Deep dive into a specific package — metadata, internal deps, dependents, public API surface, file tree |
| `explore-area` | `path` | Explore a directory subtree — packages, files, and symbol summaries under a path prefix |
| `onboard` | — | Repository overview for onboarding — tech stack, package counts by language, file distribution, index freshness |
| `impact-analysis` | `name` | Blast radius analysis — direct dependents, transitive dependents, full dependency chain |
| `understand-dependency` | `from`, `to` | Trace the dependency path between two packages |
