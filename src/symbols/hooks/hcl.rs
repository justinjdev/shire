use super::{LanguageHooks, SymbolInfo, SymbolKind, find_child_by_kind, node_text};
use tree_sitter::Node;

/// Recognized HCL block types that produce symbols.
const BLOCK_TYPES_TWO_LABELS: &[&str] = &["resource", "data"];
const BLOCK_TYPES_ONE_LABEL: &[&str] = &["variable", "output", "module", "provider"];

/// Extract the text content from an HCL string_lit node.
/// string_lit contains: quoted_template_start `"`, template_literal, quoted_template_end `"`
fn string_lit_text<'a>(node: &Node, source: &'a str) -> Option<&'a str> {
    find_child_by_kind(node, "template_literal").and_then(|n| node_text(&n, source))
}

/// Collect label strings from a block node's string_lit children.
fn collect_labels<'a>(node: &Node, source: &'a str) -> Vec<&'a str> {
    let mut labels = Vec::new();
    for i in 0..node.child_count() {
        let child = node.child(i).unwrap();
        if child.kind() == "string_lit"
            && let Some(text) = string_lit_text(&child, source)
        {
            labels.push(text);
        }
    }
    labels
}

/// Build signature: block header text from start up to (but not including) the body.
fn build_signature(node: &Node, source: &str, _name: &str, _kind: SymbolKind) -> String {
    let start = node.start_byte();
    let end = find_child_by_kind(node, "block_start")
        .map(|n| n.start_byte())
        .or_else(|| find_child_by_kind(node, "body").map(|n| n.start_byte()))
        .unwrap_or(node.end_byte());
    source[start..end.min(source.len())].trim().to_string()
}

/// Post-process: filter by block type, rewrite name and kind.
fn post_process(mut sym: SymbolInfo, node: &Node, source: &str) -> Option<SymbolInfo> {
    let block_type = sym.name.clone();
    let labels = collect_labels(node, source);

    if BLOCK_TYPES_TWO_LABELS.contains(&block_type.as_str()) {
        if labels.len() >= 2 {
            sym.name = format!("{}.{}", labels[0], labels[1]);
            sym.kind = SymbolKind::Class;
        } else {
            return None;
        }
    } else if BLOCK_TYPES_ONE_LABEL.contains(&block_type.as_str()) {
        let label = labels.first()?;
        sym.name = label.to_string();
        sym.kind = match block_type.as_str() {
            "variable" | "output" => SymbolKind::Type,
            _ => SymbolKind::Class,
        };
    } else {
        return None;
    }

    Some(sym)
}

pub fn hooks() -> LanguageHooks {
    LanguageHooks {
        build_signature: Some(build_signature),
        post_process: Some(post_process),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::symbols::SymbolKind;
    use crate::symbols::registry::extract_file;

    fn extract(source: &str) -> Vec<crate::symbols::SymbolInfo> {
        extract_file("tf", source, Arc::from("test.tf"), true, 0).0
    }

    #[test]
    fn test_resource_block() {
        let source = r#"resource "aws_instance" "web" {
  ami           = "ami-123456"
  instance_type = "t2.micro"
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "aws_instance.web");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
        assert!(symbols[0].signature.as_ref().unwrap().contains("resource"));
    }

    #[test]
    fn test_variable_block() {
        let source = r#"variable "vpc_cidr" {
  type    = string
  default = "10.0.0.0/16"
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "vpc_cidr");
        assert_eq!(symbols[0].kind, SymbolKind::Type);
        assert!(symbols[0].signature.as_ref().unwrap().contains("variable"));
    }

    #[test]
    fn test_output_block() {
        let source = r#"output "instance_ip" {
  value = aws_instance.web.public_ip
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "instance_ip");
        assert_eq!(symbols[0].kind, SymbolKind::Type);
    }

    #[test]
    fn test_data_source() {
        let source = r#"data "aws_ami" "ubuntu" {
  most_recent = true
  owners      = ["099720109477"]
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "aws_ami.ubuntu");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
        assert!(symbols[0].signature.as_ref().unwrap().contains("data"));
    }

    #[test]
    fn test_module_block() {
        let source = r#"module "vpc" {
  source = "./modules/vpc"
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "vpc");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
    }

    #[test]
    fn test_provider_block() {
        let source = r#"provider "aws" {
  region = "us-west-2"
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "aws");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
    }

    #[test]
    fn test_multiple_blocks() {
        let source = r#"variable "region" {
  default = "us-east-1"
}

resource "aws_vpc" "main" {
  cidr_block = var.vpc_cidr
}

output "vpc_id" {
  value = aws_vpc.main.id
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 3);
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_ref()).collect();
        assert!(names.contains(&"region"));
        assert!(names.contains(&"aws_vpc.main"));
        assert!(names.contains(&"vpc_id"));
    }

    #[test]
    fn test_locals_and_terraform_skipped() {
        let source = r#"locals {
  name = "test"
}

terraform {
  required_version = ">= 1.0"
}"#;
        let symbols = extract(source);
        assert_eq!(symbols.len(), 0);
    }

    #[test]
    fn test_hcl_extension() {
        let source = r#"variable "name" {
  type = string
}"#;
        let symbols = extract_file("hcl", source, Arc::from("test.hcl"), true, 0).0;
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "name");
    }
}
