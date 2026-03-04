# MCP Prompts

Prompts are pre-built templates for semantic codebase exploration. They compose multiple queries into structured context, giving your AI a map of where concepts live in the codebase.

| Prompt | Args | Description |
|---|---|---|
| `explore` | `query` | Search packages, symbols, and files for a concept — returns a structured context map organized by package |
| `explore-package` | `name` | Deep dive into a specific package — metadata, internal deps, dependents, public API surface, file tree |
| `impact-analysis` | `name` | Blast radius analysis — direct dependents, transitive dependents, full dependency chain |
