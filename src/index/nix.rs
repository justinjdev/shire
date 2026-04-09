use super::manifest::{DepInfo, DepKind, ManifestParser, PackageInfo};
use anyhow::Result;
use std::collections::HashSet;
use std::path::Path;

pub struct FlakeNixParser;

impl ManifestParser for FlakeNixParser {
    fn filename(&self) -> &'static str {
        "flake.nix"
    }

    fn parse(&self, manifest_path: &Path, relative_dir: &str) -> Result<PackageInfo> {
        let content = std::fs::read_to_string(manifest_path)?;
        let dependencies = parse_flake_nix(&content);

        let path = relative_dir.to_string();
        let name = if path.is_empty() {
            ".".to_string()
        } else {
            path.clone()
        };

        Ok(PackageInfo {
            name,
            path,
            kind: "nix",
            version: None,
            description: None,
            metadata: None,
            dependencies,
        })
    }
}

/// Parse a flake.nix file and extract its flake inputs.
///
/// Handles two common forms:
/// - Dotted form: `inputs.NAME.url = "...";` (also `inputs.NAME = { url = "..."; ... };`)
/// - Block form: `inputs = { NAME.url = "..."; NAME = { url = "..."; ... }; };`
///
/// The URL value (if present) is stored as `version_req`. Overrides such as
/// `inputs.NAME.inputs.X.follows = "Y"` only register NAME (they are not new inputs).
fn parse_flake_nix(content: &str) -> Vec<DepInfo> {
    let text = strip_comments(content);

    // Preserve first-seen order for deterministic output.
    let mut order: Vec<(String, Option<String>)> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    // A flake.nix is an attrset `{ ... }` at the top of the file. Only
    // assignments at depth 0 of that outer attrset are real flake attributes;
    // any `inputs = { ... }` or `inputs.X` occurrences inside an individual
    // input's override body are transitive-input overrides, not new direct
    // inputs. Scan only the outer body to avoid false edges.
    let Some(outer_body) = outer_attrset_body(&text) else {
        return Vec::new();
    };

    scan_top_level_inputs(outer_body, &mut order, &mut seen);

    order
        .into_iter()
        .map(|(name, url)| DepInfo {
            name,
            version_req: url,
            dep_kind: DepKind::Runtime,
        })
        .collect()
}

/// Return the slice inside the outermost `{ ... }` of the flake file, or None
/// if the file has no top-level attrset.
fn outer_attrset_body(text: &str) -> Option<&str> {
    let open = text.find('{')?;
    let close = find_matching_brace(text, open)?;
    Some(&text[open + 1..close])
}

/// Walk a flake's outer attrset body at depth 0, dispatching `inputs = { ... }`
/// and `inputs.NAME ...` occurrences. Nested `{ ... }`, `[ ... ]`, `( ... )`,
/// and string literals are skipped by depth tracking, so overrides inside
/// individual input attrsets are ignored.
fn scan_top_level_inputs(
    body: &str,
    order: &mut Vec<(String, Option<String>)>,
    seen: &mut HashSet<String>,
) {
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;

    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_string = true;
                i += 1;
                continue;
            }
            b'{' | b'[' | b'(' => {
                depth += 1;
                i += 1;
                continue;
            }
            b'}' | b']' | b')' => {
                depth -= 1;
                i += 1;
                continue;
            }
            _ => {}
        }

        if depth != 0 {
            i += 1;
            continue;
        }

        // At depth 0 of the outer flake body: look for the word `inputs`
        // followed (after whitespace) by either `=` or `.`.
        if c == b'i'
            && body[i..].starts_with("inputs")
            && is_word_boundary_before(body, i)
            && is_word_boundary_at(body, i + 6)
        {
            let mut j = i + 6;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                // `inputs = ...`
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < bytes.len()
                    && bytes[k] == b'{'
                    && let Some(close) = find_matching_brace(body, k)
                {
                    parse_inputs_body(&body[k + 1..close], order, seen);
                    i = close + 1;
                    continue;
                }
                i = j;
                continue;
            } else if j < bytes.len() && bytes[j] == b'.' {
                // `inputs.NAME...`
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                let name_start = k;
                while k < bytes.len() && is_ident_byte(bytes[k]) {
                    k += 1;
                }
                if k > name_start {
                    let name = body[name_start..k].to_string();
                    let url = extract_url_for_dotted(body, k);
                    if seen.insert(name.clone()) {
                        order.push((name, url));
                    } else if url.is_some()
                        && let Some(entry) = order.iter_mut().find(|(n, _)| n == &name)
                        && entry.1.is_none()
                    {
                        entry.1 = url;
                    }
                    // Advance just past NAME; depth tracking will walk over
                    // any `= { ... }` attrset or `.key = value` tail.
                    i = k;
                    continue;
                }
            }
        }

        i += 1;
    }
}

/// Extract URL for a top-level `inputs.NAME...` occurrence, where `pos` is the
/// byte offset immediately after `inputs.NAME`.
fn extract_url_for_dotted(text: &str, pos: usize) -> Option<String> {
    let rest = text.get(pos..)?;
    let rest = rest.trim_start();
    if let Some(after_dot) = rest.strip_prefix('.') {
        // inputs.NAME.<key> = ...
        let after_dot = after_dot.trim_start();
        // Read identifier
        let key_end = after_dot
            .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '\''))
            .unwrap_or(after_dot.len());
        let key = &after_dot[..key_end];
        if key == "url" {
            let after_key = after_dot[key_end..].trim_start();
            if let Some(eq_rest) = after_key.strip_prefix('=') {
                return extract_url_value(eq_rest.trim_start());
            }
        }
        None
    } else if let Some(after_eq) = rest.strip_prefix('=') {
        // inputs.NAME = { ... }
        let after_eq = after_eq.trim_start();
        if after_eq.starts_with('{') {
            // Find position of '{' in original text
            let brace_offset =
                pos + (text[pos..].len() - rest.len()) + (rest.len() - after_eq.len());
            if let Some(close) = find_matching_brace(text, brace_offset) {
                let body = &text[brace_offset + 1..close];
                return find_url_in_attrset(body);
            }
        }
        None
    } else {
        None
    }
}

/// Find `url = "..."` at the top level of an attrset body. Skips nested
/// attrsets entirely so that `inputs.X.url = "..."` inside a `follows` override
/// doesn't get picked up.
fn find_url_in_attrset(body: &str) -> Option<String> {
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;
    let mut depth: i32 = 0;

    // Use regex on depth-0 slices between brace transitions — simpler approach:
    // scan for `url` keyword at depth 0, followed by `=` and a quoted string.
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => {
                in_string = true;
                i += 1;
            }
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth -= 1;
                i += 1;
            }
            _ => {
                if depth == 0
                    && c == b'u'
                    && body[i..].starts_with("url")
                    && is_word_boundary_before(body, i)
                {
                    let after_url = i + 3;
                    if after_url <= body.len() && is_word_boundary_at(body, after_url) {
                        let rest = body[after_url..].trim_start();
                        if let Some(after_eq) = rest.strip_prefix('=')
                            && let Some(val) = extract_url_value(after_eq.trim_start())
                        {
                            return Some(val);
                        }
                    }
                }
                i += 1;
            }
        }
    }
    None
}

/// Parse the body of an `inputs = { ... }` block, extracting each input.
fn parse_inputs_body(
    body: &str,
    order: &mut Vec<(String, Option<String>)>,
    seen: &mut HashSet<String>,
) {
    let bytes = body.as_bytes();
    let mut i = 0;
    // Iterate entries at depth 0 (relative to the block).
    // Each entry starts with an identifier.
    while i < bytes.len() {
        // Skip whitespace and semicolons.
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b';') {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Read identifier
        let start = i;
        while i < bytes.len() && is_ident_byte(bytes[i]) {
            i += 1;
        }
        if i == start {
            // Not an identifier — skip one char to avoid infinite loop.
            i += 1;
            continue;
        }
        let name = body[start..i].to_string();

        // After name, expect either `.KEY = ...;` or `= { ... };`
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        let mut url: Option<String> = None;

        if bytes[i] == b'.' {
            // NAME.<key> = ...
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let kstart = i;
            while i < bytes.len() && is_ident_byte(bytes[i]) {
                i += 1;
            }
            let key = &body[kstart..i];
            // Skip to value terminator `;`, but if key == "url" capture value.
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'=' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if key == "url" {
                    url = extract_url_value(&body[i..]);
                }
            }
            // Skip to next `;` at depth 0.
            i = skip_to_statement_end(body, i);
        } else if bytes[i] == b'=' {
            // NAME = ...
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'{' {
                // NAME = { ... };
                if let Some(close) = find_matching_brace(body, i) {
                    let inner = &body[i + 1..close];
                    url = find_url_in_attrset(inner);
                    i = close + 1;
                } else {
                    break;
                }
            }
            i = skip_to_statement_end(body, i);
        } else {
            // Unexpected — skip to next statement end.
            i = skip_to_statement_end(body, i);
            continue;
        }

        if seen.insert(name.clone()) {
            order.push((name, url));
        } else if url.is_some() {
            // If we already saw this input but didn't have a URL, update it.
            if let Some(entry) = order.iter_mut().find(|(n, _)| n == &name)
                && entry.1.is_none()
            {
                entry.1 = url;
            }
        }
    }
}

/// Find the matching `}` for a `{` at `open`. Returns the byte index of the `}`.
fn find_matching_brace(s: &str, open: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if open >= bytes.len() || bytes[open] != b'{' {
        return None;
    }
    let mut depth: i32 = 0;
    let mut i = open;
    let mut in_string = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Advance past the next statement-ending `;` at depth 0 relative to the start position.
fn skip_to_statement_end(s: &str, start: usize) -> usize {
    let bytes = s.as_bytes();
    let mut i = start;
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'{' | b'[' | b'(' => depth += 1,
            b'}' | b']' | b')' => {
                if depth == 0 {
                    return i;
                }
                depth -= 1;
            }
            b';' if depth == 0 => return i + 1,
            _ => {}
        }
        i += 1;
    }
    i
}

/// Extract a URL value, accepting either a double-quoted string literal or an
/// unquoted Nix URI literal (e.g. `github:NixOS/nixpkgs/nixos-unstable`).
/// Nix supports both forms for flake input URLs.
fn extract_url_value(s: &str) -> Option<String> {
    let s = s.trim_start();
    if s.as_bytes().first() == Some(&b'"') {
        extract_quoted_string(s)
    } else {
        extract_uri_literal(s)
    }
}

/// Parse an unquoted Nix URI literal per the Nix lexer grammar:
/// `[A-Za-z][A-Za-z0-9+\-.]*:[A-Za-z0-9%/?:@&=+$,\-_.!~*']+`
fn extract_uri_literal(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    // Scheme: one or more of [A-Za-z0-9+-.] after the first letter.
    let mut i = 1;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || c == b'+' || c == b'-' || c == b'.' {
            i += 1;
        } else {
            break;
        }
    }
    if i >= bytes.len() || bytes[i] != b':' {
        return None;
    }
    i += 1; // consume ':'
    let body_start = i;
    while i < bytes.len() {
        let c = bytes[i];
        let ok = c.is_ascii_alphanumeric()
            || matches!(
                c,
                b'%' | b'/'
                    | b'?'
                    | b':'
                    | b'@'
                    | b'&'
                    | b'='
                    | b'+'
                    | b'$'
                    | b','
                    | b'-'
                    | b'_'
                    | b'.'
                    | b'!'
                    | b'~'
                    | b'*'
                    | b'\''
            );
        if ok {
            i += 1;
        } else {
            break;
        }
    }
    if i == body_start {
        return None;
    }
    Some(s[..i].to_string())
}

/// Extract a double-quoted string starting at `s[0..]`, returning the unquoted value.
fn extract_quoted_string(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes[0] != b'"' {
        return None;
    }
    let mut out = String::new();
    let mut i = 1;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if escape {
            match c {
                b'n' => out.push('\n'),
                b't' => out.push('\t'),
                b'r' => out.push('\r'),
                _ => out.push(c as char),
            }
            escape = false;
            i += 1;
            continue;
        }
        if c == b'\\' {
            escape = true;
            i += 1;
            continue;
        }
        if c == b'"' {
            return Some(out);
        }
        out.push(c as char);
        i += 1;
    }
    None
}

/// Strip `#` line comments and `/* */` block comments outside string literals.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    let mut escape = false;
    while i < bytes.len() {
        let c = bytes[i];
        if in_string {
            out.push(c as char);
            if escape {
                escape = false;
            } else if c == b'\\' {
                escape = true;
            } else if c == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }
        if c == b'"' {
            in_string = true;
            out.push('"');
            i += 1;
            continue;
        }
        if c == b'#' {
            // Skip to end of line (but preserve newline).
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            // Skip block comment.
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        out.push(c as char);
        i += 1;
    }
    out
}

fn is_ident_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'\''
}

fn is_word_boundary_before(s: &str, i: usize) -> bool {
    if i == 0 {
        return true;
    }
    let prev = s.as_bytes()[i - 1];
    !is_ident_byte(prev)
}

fn is_word_boundary_at(s: &str, i: usize) -> bool {
    if i >= s.len() {
        return true;
    }
    !is_ident_byte(s.as_bytes()[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("flake.nix");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn test_parse_block_form() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  description = "A basic flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }: { };
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "infra/nix").unwrap();

        assert_eq!(info.name, "infra/nix");
        assert_eq!(info.kind, "nix");
        assert_eq!(info.path, "infra/nix");
        assert_eq!(info.dependencies.len(), 2);

        assert_eq!(info.dependencies[0].name, "nixpkgs");
        assert_eq!(
            info.dependencies[0].version_req.as_deref(),
            Some("github:NixOS/nixpkgs/nixos-unstable")
        );
        assert!(matches!(info.dependencies[0].dep_kind, DepKind::Runtime));

        assert_eq!(info.dependencies[1].name, "flake-utils");
        assert_eq!(
            info.dependencies[1].version_req.as_deref(),
            Some("github:numtide/flake-utils")
        );
    }

    #[test]
    fn test_parse_dotted_form() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  inputs.home-manager.url = "github:nix-community/home-manager";

  outputs = { self, nixpkgs, home-manager }: { };
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, ".").unwrap();

        assert_eq!(info.dependencies.len(), 2);
        let names: Vec<&str> = info.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"nixpkgs"));
        assert!(names.contains(&"home-manager"));

        let nixpkgs = info
            .dependencies
            .iter()
            .find(|d| d.name == "nixpkgs")
            .unwrap();
        assert_eq!(
            nixpkgs.version_req.as_deref(),
            Some("github:NixOS/nixpkgs/nixos-unstable")
        );
    }

    #[test]
    fn test_parse_block_with_nested_attrset() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "proj").unwrap();

        assert_eq!(info.dependencies.len(), 2);

        let nixpkgs = info
            .dependencies
            .iter()
            .find(|d| d.name == "nixpkgs")
            .unwrap();
        assert_eq!(
            nixpkgs.version_req.as_deref(),
            Some("github:NixOS/nixpkgs/nixos-unstable")
        );

        let rust = info
            .dependencies
            .iter()
            .find(|d| d.name == "rust-overlay")
            .unwrap();
        assert_eq!(
            rust.version_req.as_deref(),
            Some("github:oxalica/rust-overlay")
        );
    }

    #[test]
    fn test_parse_dotted_with_nested_attrset() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.nixpkgs.follows = "nixpkgs";
  };
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "proj").unwrap();

        assert_eq!(info.dependencies.len(), 2);
        let rust = info
            .dependencies
            .iter()
            .find(|d| d.name == "rust-overlay")
            .unwrap();
        assert_eq!(
            rust.version_req.as_deref(),
            Some("github:oxalica/rust-overlay")
        );
        let nixpkgs = info
            .dependencies
            .iter()
            .find(|d| d.name == "nixpkgs")
            .unwrap();
        assert_eq!(
            nixpkgs.version_req.as_deref(),
            Some("github:NixOS/nixpkgs/nixos-unstable")
        );
    }

    #[test]
    fn test_parse_skips_comments() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  # inputs.fake.url = "should-be-ignored";
  inputs = {
    /* nixpkgs.url = "also-ignored"; */
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "x").unwrap();

        assert_eq!(info.dependencies.len(), 1);
        assert_eq!(info.dependencies[0].name, "nixpkgs");
        assert!(
            info.dependencies
                .iter()
                .all(|d| d.name != "fake" && d.name != "nixpkgs-also")
        );
    }

    #[test]
    fn test_parse_empty_flake() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  description = "no inputs";
  outputs = { self }: { };
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "empty").unwrap();

        assert_eq!(info.name, "empty");
        assert_eq!(info.kind, "nix");
        assert!(info.dependencies.is_empty());
    }

    #[test]
    fn test_parse_input_without_url() {
        // Some inputs may only appear as `inputs.NAME.flake = false;` — record name only.
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  inputs.local-src.url = "path:./vendor";
  inputs.local-src.flake = false;
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, ".").unwrap();

        assert_eq!(info.dependencies.len(), 1);
        assert_eq!(info.dependencies[0].name, "local-src");
        assert_eq!(
            info.dependencies[0].version_req.as_deref(),
            Some("path:./vendor")
        );
    }

    #[test]
    fn test_nested_inputs_overrides_not_treated_as_direct_deps() {
        // Nested transitive-input overrides must not be registered as direct
        // flake inputs. Here `inputs.bar.follows` lives inside rust-overlay
        // and should not produce a top-level `bar` edge.
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  inputs.rust-overlay = {
    url = "github:oxalica/rust-overlay";
    inputs.bar.follows = "nixpkgs";
  };
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "proj").unwrap();

        let names: Vec<&str> = info.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(
            info.dependencies.len(),
            2,
            "expected only top-level inputs, got {names:?}"
        );
        assert!(names.contains(&"rust-overlay"));
        assert!(names.contains(&"nixpkgs"));
        assert!(
            !names.contains(&"bar"),
            "nested inputs.bar.follows override was incorrectly registered as a direct dep"
        );
    }

    #[test]
    fn test_nested_inputs_block_not_treated_as_direct_deps() {
        // Nested `inputs = { ... }` attrsets (inside another input) must not
        // leak their entries as top-level flake inputs.
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  inputs.foo = {
    url = "github:x/foo";
    inputs = {
      bar.follows = "nixpkgs";
    };
  };
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "proj").unwrap();

        let names: Vec<&str> = info.dependencies.iter().map(|d| d.name.as_str()).collect();
        assert!(!names.contains(&"bar"), "got {names:?}");
    }

    #[test]
    fn test_root_flake_uses_empty_path() {
        // A root flake.nix (relative_dir == "") must be keyed by "" to match
        // how the rest of the indexer keys repo-root packages. `name` may be
        // "." for display, but `path` must mirror relative_dir or file
        // association, delete, and stale-hash cleanup flows will silently skip
        // the root package.
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "").unwrap();

        assert_eq!(info.path, "");
        assert_eq!(info.name, ".");
    }

    #[test]
    fn test_parse_unquoted_uri_literal_dotted() {
        // Nix allows unquoted URI literals for flake URLs.
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  inputs.nixpkgs.url = github:NixOS/nixpkgs/nixos-unstable;
  inputs.flake-utils.url = github:numtide/flake-utils;
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "proj").unwrap();

        assert_eq!(info.dependencies.len(), 2);
        let nixpkgs = info
            .dependencies
            .iter()
            .find(|d| d.name == "nixpkgs")
            .unwrap();
        assert_eq!(
            nixpkgs.version_req.as_deref(),
            Some("github:NixOS/nixpkgs/nixos-unstable")
        );
        let utils = info
            .dependencies
            .iter()
            .find(|d| d.name == "flake-utils")
            .unwrap();
        assert_eq!(
            utils.version_req.as_deref(),
            Some("github:numtide/flake-utils")
        );
    }

    #[test]
    fn test_parse_unquoted_uri_literal_block() {
        let dir = TempDir::new().unwrap();
        let path = write_manifest(
            dir.path(),
            r#"{
  inputs = {
    nixpkgs.url = github:NixOS/nixpkgs/nixos-unstable;
    rust-overlay = {
      url = github:oxalica/rust-overlay;
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };
}
"#,
        );

        let parser = FlakeNixParser;
        let info = parser.parse(&path, "proj").unwrap();

        assert_eq!(info.dependencies.len(), 2);
        let nixpkgs = info
            .dependencies
            .iter()
            .find(|d| d.name == "nixpkgs")
            .unwrap();
        assert_eq!(
            nixpkgs.version_req.as_deref(),
            Some("github:NixOS/nixpkgs/nixos-unstable")
        );
        let rust = info
            .dependencies
            .iter()
            .find(|d| d.name == "rust-overlay")
            .unwrap();
        assert_eq!(
            rust.version_req.as_deref(),
            Some("github:oxalica/rust-overlay")
        );
    }

    #[test]
    fn test_extract_uri_literal_accepts_common_schemes() {
        assert_eq!(
            extract_uri_literal("github:NixOS/nixpkgs"),
            Some("github:NixOS/nixpkgs".to_string())
        );
        assert_eq!(
            extract_uri_literal("https://example.com/foo.tar.gz"),
            Some("https://example.com/foo.tar.gz".to_string())
        );
        assert_eq!(
            extract_uri_literal("path:./vendor"),
            Some("path:./vendor".to_string())
        );
    }

    #[test]
    fn test_extract_uri_literal_rejects_non_uri() {
        // Plain identifiers or numbers are not URIs.
        assert_eq!(extract_uri_literal("nixpkgs"), None);
        assert_eq!(extract_uri_literal("1github:foo"), None);
        // A colon with no body is not a URI.
        assert_eq!(extract_uri_literal("github:"), None);
    }

    #[test]
    fn test_find_matching_brace() {
        let s = "{ a { b } c }";
        assert_eq!(find_matching_brace(s, 0), Some(12));
        assert_eq!(find_matching_brace(s, 4), Some(8));
    }

    #[test]
    fn test_find_matching_brace_string_aware() {
        let s = r#"{ url = "a{b}c"; }"#;
        // The outer { at position 0, should find } at position 17
        assert_eq!(find_matching_brace(s, 0), Some(17));
    }

    #[test]
    fn test_strip_comments_preserves_strings() {
        let s = r##"x = "# not a comment"; # but this is"##;
        let out = strip_comments(s);
        assert!(out.contains("# not a comment"));
        assert!(!out.contains("but this is"));
    }
}
