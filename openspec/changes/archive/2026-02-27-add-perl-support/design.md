## Context

Perl has no Shire support — no package discovery and no symbol extraction. Packages use `cpanfile` for dependency declaration, which lists runtime and test dependencies in a simple DSL (`requires 'Module::Name', '1.0';`). Symbol extraction covers `sub` definitions and `package` declarations in `.pm`/`.pl` files.

This change adds both a `ManifestParser` implementation for `cpanfile` and a tree-sitter symbol extractor.

## Goals / Non-Goals

**Goals:**
- Discover Perl packages from `cpanfile` manifest files
- Extract symbols from `.pm` and `.pl` files (subs, package declarations)
- Add `cpanfile` to the default manifest list

**Non-Goals:**
- Support for `Makefile.PL`, `Build.PL`, or `META.json` (can be added later)
- Perl dependency version resolution or CPAN integration
- Extracting symbols from inline scripts or one-liners
- Moose/Moo attribute extraction (would require framework-specific parsing)

## Decisions

### 1. cpanfile as the manifest

`cpanfile` is the modern standard for Perl dependency declaration. It's a Perl DSL but the common patterns are parseable with regex:

```perl
requires 'DBI', '>= 1.600';
requires 'JSON::XS';
on 'test' => sub {
    requires 'Test::More', '0.88';
};
```

Parse `requires 'Name'` lines for runtime deps, `on 'test'` blocks for dev deps. Skip complex Perl expressions — if a cpanfile uses dynamic logic, those deps are silently missed. This matches the pragmatic approach of other parsers (e.g., Gradle regex parsing).

### 2. Package naming from cpanfile

Unlike npm's `package.json` which has an explicit `name` field, `cpanfile` has no package name. Use the directory name (relative to repo root) as the package name, consistent with how Gradle packages without `rootProject.name` are named. Set `kind` to `"perl"`.

### 3. Symbol extraction: subs and packages

| Perl construct | SymbolKind | Notes |
|---|---|---|
| `sub name { ... }` | `Function` | Top-level subroutine |
| `sub name { ... }` inside a `package` block | `Method` | With `parent_symbol` = package name |
| `package Foo::Bar;` | `Class` | Perl packages are the closest analog to classes |

**Visibility:** Perl has no formal visibility keywords. By convention, subs starting with `_` are private. Extract all subs except those prefixed with `_`.

### 4. tree-sitter-perl

Use `tree-sitter-perl` for parsing. Perl syntax is notoriously complex but tree-sitter-perl handles common patterns well. Fallback to regex if the crate is unmaintained — `sub\s+(\w+)` and `package\s+([\w:]+)` cover the majority of cases.

## Risks / Trade-offs

**[Parser quality] tree-sitter-perl may not handle all Perl idioms** → Mitigation: Extraction resilience spec already covers unparseable files (skip silently). Perl's dynamic features (string eval, autoload) are inherently unextractable by any static tool.

**[cpanfile complexity] Some cpanfiles use Perl logic beyond simple `requires`** → Mitigation: Parse what we can, skip what we can't. Same approach as the Gradle regex parser.

**[No package name in cpanfile]** → Mitigation: Directory-derived naming is consistent with other parsers' fallback behavior. Users can override via `[[packages]]` config.
