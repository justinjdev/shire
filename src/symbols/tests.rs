use super::*;
use std::sync::Arc;

// ============================================================
// Python tests (ported from python.rs)
// ============================================================

#[test]
fn test_python_function_with_type_hints() {
    let source = r#"def process_payment(amount: float, currency: str) -> Receipt:
    pass
"#;
    let (symbols, _) = extract_file("py", source, Arc::from("pay.py"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "process_payment");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert_eq!(sym.return_type.as_deref(), Some("Receipt"));
    let params = sym.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "amount");
    assert_eq!(params[0].type_annotation.as_deref(), Some("float"));
    assert_eq!(params[1].name, "currency");
    assert_eq!(params[1].type_annotation.as_deref(), Some("str"));
}

#[test]
fn test_python_class_with_methods() {
    let source = r#"class AuthService:
    def __init__(self, db: Database):
        self.db = db

    def validate(self, token: str) -> bool:
        return True

    def _internal(self):
        pass
"#;
    let (symbols, _) = extract_file("py", source, Arc::from("auth.py"), true, 0);
    // class + __init__ + validate (skip _internal)
    assert_eq!(symbols.len(), 3);
    assert_eq!(symbols[0].name, "AuthService");
    assert_eq!(symbols[0].kind, SymbolKind::Class);

    assert_eq!(symbols[1].name, "__init__");
    assert_eq!(symbols[1].kind, SymbolKind::Method);
    assert_eq!(symbols[1].parent_symbol.as_deref(), Some("AuthService"));
    // self should be filtered out
    let init_params = symbols[1].parameters.as_ref().unwrap();
    assert_eq!(init_params.len(), 1);
    assert_eq!(init_params[0].name, "db");

    assert_eq!(symbols[2].name, "validate");
    assert_eq!(symbols[2].kind, SymbolKind::Method);

    assert!(!symbols.iter().any(|s| s.name == "_internal"));
}

#[test]
fn test_python_decorated_functions_and_methods_are_extracted() {
    // SYM-1: a decorated def is wrapped in `decorated_definition`, which the
    // undecorated-only query patterns did not match at all.
    let source = r#"from dataclasses import dataclass

def plain_function(x):
    return x

@app.route("/users")
def list_users():
    return []

@dataclass
class User:
    name: str

    @property
    def display_name(self):
        return self.name

    @staticmethod
    def make():
        return User("x")

    def plain_method(self):
        return 1

    @property
    def _hidden(self):
        return 0
"#;
    let (symbols, _) = extract_file("py", source, Arc::from("app.py"), true, 0);
    let mut got: Vec<(&str, SymbolKind, Option<&str>)> = symbols
        .iter()
        .map(|s| (s.name.as_str(), s.kind, s.parent_symbol.as_deref()))
        .collect();
    got.sort_by_key(|s| s.0);
    let mut expected: Vec<(&str, SymbolKind, Option<&str>)> = vec![
        ("plain_function", SymbolKind::Function, None),
        ("list_users", SymbolKind::Function, None),
        ("User", SymbolKind::Class, None),
        ("display_name", SymbolKind::Method, Some("User")),
        ("make", SymbolKind::Method, Some("User")),
        ("plain_method", SymbolKind::Method, Some("User")),
        // `_hidden` is decorated but still private inside a class — must stay excluded.
    ];
    expected.sort_by_key(|s| s.0);
    assert_eq!(got, expected, "got {:?}", got);

    let list_users = symbols.iter().find(|s| s.name == "list_users").unwrap();
    assert_eq!(list_users.line, 7);
    let display_name = symbols.iter().find(|s| s.name == "display_name").unwrap();
    assert_eq!(display_name.line, 15);
    let make = symbols.iter().find(|s| s.name == "make").unwrap();
    assert_eq!(make.line, 19);
}

#[test]
fn test_python_function_no_hints() {
    let source = r#"def greet(name):
    return f"Hello {name}"
"#;
    let (symbols, _) = extract_file("py", source, Arc::from("greet.py"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "greet");
    assert!(symbols[0].return_type.is_none());
}

// Python reference tests

#[test]
fn test_python_call_references() {
    let source = r#"import json

def load(path: str) -> dict:
    raw = open(path).read()
    return json.loads(raw)

def save(path: str, data: dict):
    with open(path, 'w') as f:
        f.write(json.dumps(data))
"#;
    let (_syms, refs) = extract_file("py", source, Arc::from("io.py"), false, 0);
    let mut call_refs: Vec<(String, usize, Option<String>)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| (r.name.clone(), r.line, r.enclosing_symbol.clone()))
        .collect();
    call_refs.sort();
    assert_eq!(
        call_refs,
        vec![
            ("dumps".into(), 9, Some("save".into())),
            ("loads".into(), 5, Some("load".into())),
            ("read".into(), 4, Some("load".into())),
            ("write".into(), 9, Some("save".into())),
        ]
    );
}

#[test]
fn test_python_import_references() {
    let source = r#"import json
from typing import List, Dict
from os.path import join
"#;
    let (_syms, refs) = extract_file("py", source, Arc::from("imports.py"), false, 0);
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"json"), "got {:?}", names);
    assert!(names.contains(&"List"));
    assert!(names.contains(&"Dict"));
    assert!(names.contains(&"join"));
}

#[test]
fn test_python_type_references() {
    let source = r#"class Config:
    pass

def build(req: Request) -> Config:
    return Config()
"#;
    let (_syms, refs) = extract_file("py", source, Arc::from("types.py"), false, 0);
    let mut type_refs: Vec<(String, usize)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Type)
        .map(|r| (r.name.clone(), r.line))
        .collect();
    type_refs.sort();
    assert_eq!(type_refs, vec![("Config".into(), 4), ("Request".into(), 4)]);
}

#[test]
fn test_python_impl_references() {
    let source = r#"class Base:
    pass

class Derived(Base):
    pass

class Multi(Base, Mixin):
    pass
"#;
    let (_syms, refs) = extract_file("py", source, Arc::from("cls.py"), false, 0);
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"Base"), "got {:?}", names);
    assert!(names.contains(&"Mixin"));
}

#[test]
fn test_python_lambda_call_uses_outer_function_as_enclosing() {
    let source = r#"def transform(items):
    return list(map(lambda x: normalize(x), items))
"#;
    let (_syms, refs) = extract_file("py", source, Arc::from("lambda.py"), false, 0);
    let normalize_call = refs
        .iter()
        .find(|r| r.kind == ReferenceKind::Call && r.name == "normalize")
        .unwrap();
    assert_eq!(normalize_call.line, 2);
    assert_eq!(
        normalize_call.enclosing_symbol.as_deref(),
        Some("transform")
    );
}

// ============================================================
// Go tests (ported from go.rs)
// ============================================================

#[test]
fn test_go_exported_function() {
    let source = r#"package main

func ProcessPayment(amount float64, currency string) (*Receipt, error) {
    return nil, nil
}
"#;
    let (symbols, _) = extract_file("go", source, Arc::from("handler.go"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "ProcessPayment");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(sym.signature.as_ref().unwrap().contains("ProcessPayment"));
    assert_eq!(sym.return_type.as_deref(), Some("(*Receipt, error)"));
    let params = sym.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "amount");
    assert_eq!(params[0].type_annotation.as_deref(), Some("float64"));
}

#[test]
fn test_go_exported_struct() {
    let source = r#"package main

type AuthService struct {
    db *sql.DB
}
"#;
    let (symbols, _) = extract_file("go", source, Arc::from("auth.go"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "AuthService");
    assert_eq!(symbols[0].kind, SymbolKind::Struct);
}

#[test]
fn test_go_exported_interface() {
    let source = r#"package main

type Handler interface {
    ServeHTTP(w ResponseWriter, r *Request)
}
"#;
    let (symbols, _) = extract_file("go", source, Arc::from("handler.go"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Handler");
    assert_eq!(symbols[0].kind, SymbolKind::Interface);
}

#[test]
fn test_go_method_with_receiver() {
    let source = r#"package main

func (s *AuthService) Validate(token string) error {
    return nil
}
"#;
    let (symbols, _) = extract_file("go", source, Arc::from("auth.go"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Validate");
    assert_eq!(symbols[0].kind, SymbolKind::Method);
    assert_eq!(symbols[0].parent_symbol.as_deref(), Some("AuthService"));
}

#[test]
fn test_go_skip_unexported() {
    let source = r#"package main

func internalHelper() {}
type internalType struct {}
"#;
    let (symbols, _) = extract_file("go", source, Arc::from("internal.go"), true, 0);
    assert!(symbols.is_empty());
}

// References

#[test]
fn test_go_call_references() {
    let source = r#"package main

import "fmt"

func handleRequest(req *Request) error {
    cfg := parseConfig(req)
    fmt.Println(cfg)
    return validate(cfg)
}

func parseConfig(r *Request) Config { return Config{} }
func validate(c Config) error { return nil }
"#;
    let (_syms, refs) = extract_file("go", source, Arc::from("main.go"), false, 0);

    let mut call_refs: Vec<(String, usize, Option<String>)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| (r.name.clone(), r.line, r.enclosing_symbol.clone()))
        .collect();
    call_refs.sort();
    assert_eq!(
        call_refs,
        vec![
            ("Println".into(), 7, Some("handleRequest".into())),
            ("parseConfig".into(), 6, Some("handleRequest".into())),
            ("validate".into(), 8, Some("handleRequest".into())),
        ]
    );
}

#[test]
fn test_go_type_references() {
    let source = r#"package main

type Config struct { Key string }

func parse(r *Request) Config { return Config{} }
"#;
    let (_syms, refs) = extract_file("go", source, Arc::from("main.go"), false, 0);
    let type_refs: Vec<&ReferenceInfo> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Type)
        .collect();
    let names: Vec<&str> = type_refs.iter().map(|r| r.name.as_str()).collect();
    assert!(
        names.contains(&"Request"),
        "expected type-ref Request, got {:?}",
        names
    );
    assert!(
        names.contains(&"Config"),
        "expected type-ref Config, got {:?}",
        names
    );
}

#[test]
fn test_go_import_references() {
    let source = r#"package main

import (
    "fmt"
    "strings"
)
"#;
    let (_syms, refs) = extract_file("go", source, Arc::from("main.go"), false, 0);
    let import_refs: Vec<&ReferenceInfo> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .collect();
    let names: Vec<&str> = import_refs.iter().map(|r| r.name.as_str()).collect();
    assert!(
        names.contains(&"fmt"),
        "expected fmt import (quotes stripped by query_extract), got {:?}",
        names
    );
    assert!(names.contains(&"strings"), "got {:?}", names);
}

// ============================================================
// Rust tests (ported from rust_lang.rs)
// ============================================================

#[test]
fn test_rust_pub_function() {
    let source = r#"pub fn process_payment(amount: f64, currency: &str) -> Result<Receipt> {
    todo!()
}"#;
    let (symbols, _) = extract_file("rs", source, Arc::from("src/pay.rs"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "process_payment");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(
        sym.signature
            .as_ref()
            .unwrap()
            .contains("pub fn process_payment")
    );
    assert_eq!(sym.return_type.as_deref(), Some("Result<Receipt>"));
    let params = sym.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "amount");
    assert_eq!(params[0].type_annotation.as_deref(), Some("f64"));
    assert_eq!(params[1].name, "currency");
    assert_eq!(params[1].type_annotation.as_deref(), Some("&str"));
}

#[test]
fn test_rust_pub_struct() {
    let source = r#"pub struct AuthService {
    db: Connection,
}"#;
    let (symbols, _) = extract_file("rs", source, Arc::from("src/auth.rs"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "AuthService");
    assert_eq!(symbols[0].kind, SymbolKind::Struct);
}

#[test]
fn test_rust_pub_enum() {
    let source = r#"pub enum Status {
    Active,
    Inactive,
}"#;
    let (symbols, _) = extract_file("rs", source, Arc::from("src/types.rs"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Status");
    assert_eq!(symbols[0].kind, SymbolKind::Enum);
}

#[test]
fn test_rust_pub_trait() {
    let source = r#"pub trait Handler {
    fn handle(&self) -> Result<()>;
}"#;
    let (symbols, _) = extract_file("rs", source, Arc::from("src/handler.rs"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Handler");
    assert_eq!(symbols[0].kind, SymbolKind::Trait);
}

#[test]
fn test_rust_impl_method() {
    let source = r#"impl AuthService {
    pub fn validate(&self, token: &str) -> Result<()> {
        todo!()
    }

    fn internal_helper(&self) {}
}"#;
    let (symbols, _) = extract_file("rs", source, Arc::from("src/auth.rs"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "validate");
    assert_eq!(symbols[0].kind, SymbolKind::Method);
    assert_eq!(symbols[0].parent_symbol.as_deref(), Some("AuthService"));
    let params = symbols[0].parameters.as_ref().unwrap();
    assert_eq!(params.len(), 1); // self is skipped
    assert_eq!(params[0].name, "token");
}

#[test]
fn test_rust_skip_non_pub() {
    let source = r#"fn internal_fn() {}
struct InternalStruct {}
enum InternalEnum {}
"#;
    let (symbols, _) = extract_file("rs", source, Arc::from("src/internal.rs"), true, 0);
    assert!(symbols.is_empty());
}

// ============================================================
// TypeScript tests (ported from typescript.rs)
// ============================================================

#[test]
fn test_ts_exported_function() {
    let source = r#"export function processPayment(amount: number, currency: string): Promise<Receipt> {
    return fetch('/pay');
}"#;
    let (symbols, _) = extract_file("ts", source, Arc::from("src/pay.ts"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "processPayment");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(sym.signature.as_ref().unwrap().contains("processPayment"));
    assert_eq!(sym.return_type.as_deref(), Some("Promise<Receipt>"));
    let params = sym.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "amount");
    assert_eq!(params[0].type_annotation.as_deref(), Some("number"));
    assert_eq!(params[1].name, "currency");
    assert_eq!(params[1].type_annotation.as_deref(), Some("string"));
}

#[test]
fn test_ts_exported_class_with_methods() {
    let source = r#"export class AuthService {
    validate(token: string): boolean {
        return true;
    }
    private _internal(): void {}
}"#;
    let (symbols, _) = extract_file("ts", source, Arc::from("src/auth.ts"), true, 0);
    assert_eq!(symbols.len(), 2, "expected class + public method only");
    assert_eq!(symbols[0].name, "AuthService");
    assert_eq!(symbols[0].kind, SymbolKind::Class);
    assert_eq!(symbols[1].name, "validate");
    assert_eq!(symbols[1].kind, SymbolKind::Method);
    assert_eq!(symbols[1].parent_symbol.as_deref(), Some("AuthService"));
    assert!(!symbols.iter().any(|s| s.name == "_internal"));
}

#[test]
fn test_ts_exported_interface() {
    let source = r#"export interface UserConfig {
    name: string;
    theme: string;
}"#;
    let (symbols, _) = extract_file("ts", source, Arc::from("src/types.ts"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "UserConfig");
    assert_eq!(symbols[0].kind, SymbolKind::Interface);
}

#[test]
fn test_ts_exported_type_alias() {
    let source = "export type Result<T> = Success<T> | Failure;";
    let (symbols, _) = extract_file("ts", source, Arc::from("src/types.ts"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Result");
    assert_eq!(symbols[0].kind, SymbolKind::Type);
}

#[test]
fn test_ts_exported_enum() {
    let source = r#"export enum Status {
    Active,
    Inactive
}"#;
    let (symbols, _) = extract_file("ts", source, Arc::from("src/types.ts"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Status");
    assert_eq!(symbols[0].kind, SymbolKind::Enum);
}

#[test]
fn test_ts_exported_const() {
    let source = "export const MAX_RETRIES = 3;";
    let (symbols, _) = extract_file("ts", source, Arc::from("src/config.ts"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "MAX_RETRIES");
    assert_eq!(symbols[0].kind, SymbolKind::Constant);
}

#[test]
fn test_ts_skip_non_exported() {
    let source = r#"
function internalHelper() {}
class InternalClass {}
const secret = 42;
"#;
    let (symbols, _) = extract_file("ts", source, Arc::from("src/internal.ts"), true, 0);
    assert!(symbols.is_empty());
}

#[test]
fn test_ts_default_export_function() {
    let source = r#"export default function handler(req: Request): Response {
    return new Response();
}"#;
    let (symbols, _) = extract_file("ts", source, Arc::from("src/handler.ts"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "handler");
    assert_eq!(symbols[0].kind, SymbolKind::Function);
}

#[test]
fn test_js_function() {
    let source = r#"export function greet(name) {
    return 'Hello ' + name;
}"#;
    let (symbols, _) = extract_file("js", source, Arc::from("src/greet.js"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "greet");
}

#[test]
fn test_typescript_call_references() {
    let source = r#"import { parseConfig } from './config';

export function handle(req: Request): Response {
    const cfg = parseConfig(req.body);
    return buildResponse(cfg);
}

function buildResponse(cfg: Config): Response {
    return new Response();
}
"#;
    let (_syms, refs) = extract_file("ts", source, Arc::from("handler.ts"), false, 0);
    let mut call_refs: Vec<(String, usize, Option<String>)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| (r.name.clone(), r.line, r.enclosing_symbol.clone()))
        .collect();
    call_refs.sort();
    assert_eq!(
        call_refs,
        vec![
            ("buildResponse".into(), 5, Some("handle".into())),
            ("parseConfig".into(), 4, Some("handle".into())),
        ]
    );
}

#[test]
fn test_typescript_type_references() {
    let source = r#"interface Config {
    key: string;
}

function handle(req: Request): Response {
    return new Response();
}
"#;
    let (_syms, refs) = extract_file("ts", source, Arc::from("h.ts"), false, 0);
    let type_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Type)
        .map(|r| r.name.as_str())
        .collect();
    assert!(type_names.contains(&"Request"), "got {:?}", type_names);
    assert!(type_names.contains(&"Response"));
}

#[test]
fn test_typescript_default_and_namespace_imports() {
    let source = r#"import Foo from './foo';
import * as Bar from './bar';
import { Baz } from './baz';
"#;
    let (_syms, refs) = extract_file("ts", source, Arc::from("i.ts"), false, 0);
    let imp_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    assert!(
        imp_names.contains(&"Foo"),
        "default import missing — got {:?}",
        imp_names
    );
    assert!(
        imp_names.contains(&"Bar"),
        "namespace import missing — got {:?}",
        imp_names
    );
    assert!(
        imp_names.contains(&"Baz"),
        "named import missing — got {:?}",
        imp_names
    );
}

#[test]
fn test_typescript_interface_extends() {
    let source = r#"interface A {}
interface B extends A {}
"#;
    let (_syms, refs) = extract_file("ts", source, Arc::from("h.ts"), false, 0);
    let impl_refs: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(
        impl_refs.contains(&"A"),
        "interface extends not captured — got {:?}",
        impl_refs
    );
}

#[test]
fn test_typescript_impl_references() {
    let source = r#"interface Service {}
interface Auditable {}
class Base {}

class MyService extends Base implements Service, Auditable {
}
"#;
    let (_syms, refs) = extract_file("ts", source, Arc::from("svc.ts"), false, 0);
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(impl_names.contains(&"Base"), "got {:?}", impl_names);
    assert!(impl_names.contains(&"Service"));
    assert!(impl_names.contains(&"Auditable"));
}

#[test]
fn test_typescript_import_references() {
    let source = r#"import { parseConfig, Config } from './config';
import { handler } from './handler';
"#;
    let (_syms, refs) = extract_file("ts", source, Arc::from("i.ts"), false, 0);
    let imp_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    assert!(imp_names.contains(&"parseConfig"), "got {:?}", imp_names);
    assert!(imp_names.contains(&"Config"));
    assert!(imp_names.contains(&"handler"));
}

#[test]
fn test_typescript_arrow_call_references_use_anonymous_enclosing() {
    let source = r#"class Handler {
  run() {
    const f = () => parseConfig();
  }
}
"#;
    let (_syms, refs) = extract_file("ts", source, Arc::from("arrow.ts"), false, 0);
    let parse = refs
        .iter()
        .find(|r| r.kind == ReferenceKind::Call && r.name == "parseConfig")
        .unwrap();
    assert_eq!(parse.line, 3);
    // `f` is nameable from its `const f = ...` binding, and the enclosing
    // path is qualified through the containing method and class.
    assert_eq!(parse.enclosing_symbol.as_deref(), Some("Handler.run.f"));
}

#[test]
fn test_javascript_call_references() {
    let source = r#"import { parseConfig } from './config.js';

export function handle(req) {
    const cfg = parseConfig(req.body);
    return buildResponse(cfg);
}

function buildResponse(cfg) {
    return {};
}
"#;
    let (_syms, refs) = extract_file("js", source, Arc::from("h.js"), false, 0);
    let mut call_refs: Vec<(String, usize, Option<String>)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| (r.name.clone(), r.line, r.enclosing_symbol.clone()))
        .collect();
    call_refs.sort();
    assert_eq!(
        call_refs,
        vec![
            ("buildResponse".into(), 5, Some("handle".into())),
            ("parseConfig".into(), 4, Some("handle".into())),
        ]
    );
}

#[test]
fn test_javascript_impl_references() {
    let source = r#"class Base {}

class Derived extends Base {}
"#;
    let (_syms, refs) = extract_file("js", source, Arc::from("c.js"), false, 0);
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(impl_names.contains(&"Base"), "got {:?}", impl_names);
}

#[test]
fn test_javascript_import_references() {
    let source = r#"import { parseConfig, buildResponse } from './config.js';
"#;
    let (_syms, refs) = extract_file("js", source, Arc::from("imports.js"), false, 0);
    let mut import_refs: Vec<(String, usize)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| (r.name.clone(), r.line))
        .collect();
    import_refs.sort();
    assert_eq!(
        import_refs,
        vec![("buildResponse".into(), 1), ("parseConfig".into(), 1)]
    );
}

#[test]
fn test_javascript_arrow_call_references_use_anonymous_enclosing() {
    let source = r#"class Handler {
  run() {
    const f = () => parseConfig();
  }
}
"#;
    let (_syms, refs) = extract_file("js", source, Arc::from("arrow.js"), false, 0);
    let parse = refs
        .iter()
        .find(|r| r.kind == ReferenceKind::Call && r.name == "parseConfig")
        .unwrap();
    assert_eq!(parse.line, 3);
    // `f` is nameable from its `const f = ...` binding, and the enclosing
    // path is qualified through the containing method and class.
    assert_eq!(parse.enclosing_symbol.as_deref(), Some("Handler.run.f"));
}

#[test]
fn test_javascript_arrow_const_uses_binding_name_not_anonymous() {
    // Reproduces SYM-4 case A: top-level arrow functions bound to a const
    // must be individually attributable, not collapsed into "<anonymous>".
    let source = r#"export const loadUser = async (id) => { return fetchUser(id); };
export const saveUser = async (u) => { return fetchUser(u); };
export function fetchUser(id) { return id; }
"#;
    let (_syms, refs) = extract_file("js", source, Arc::from("a.js"), false, 0);
    let mut call_refs: Vec<(String, usize, Option<String>)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call && r.name == "fetchUser")
        .map(|r| (r.name.clone(), r.line, r.enclosing_symbol.clone()))
        .collect();
    call_refs.sort_by_key(|r| r.1);
    assert_eq!(
        call_refs,
        vec![
            ("fetchUser".into(), 1, Some("loadUser".into())),
            ("fetchUser".into(), 2, Some("saveUser".into())),
        ]
    );
}

#[test]
fn test_javascript_same_named_methods_on_different_classes_do_not_collapse() {
    // Reproduces SYM-4 case B: same-named methods on different classes must
    // not share one `enclosing_symbol` bucket.
    let source = r#"export class Alpha { run() { return helper(1); } }
export class Beta  { run() { return helper(2); } }
export function helper(x) { return x; }
"#;
    let (_syms, refs) = extract_file("js", source, Arc::from("c.js"), false, 0);
    let mut call_refs: Vec<(String, usize, Option<String>)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call && r.name == "helper")
        .map(|r| (r.name.clone(), r.line, r.enclosing_symbol.clone()))
        .collect();
    call_refs.sort_by_key(|r| r.1);
    assert_eq!(
        call_refs,
        vec![
            ("helper".into(), 1, Some("Alpha.run".into())),
            ("helper".into(), 2, Some("Beta.run".into())),
        ]
    );
}

// ============================================================
// Java tests (ported from java.rs)
// ============================================================

#[test]
fn test_java_public_class() {
    let source = r#"
public class UserService {
    private int count;
}
"#;
    let (symbols, _) = extract_file("java", source, Arc::from("UserService.java"), true, 0);
    let classes: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].name, "UserService");
    assert!(classes[0].signature.as_ref().unwrap().contains("class"));
    assert!(
        classes[0]
            .signature
            .as_ref()
            .unwrap()
            .contains("UserService")
    );
}

#[test]
fn test_java_public_interface() {
    let source = r#"
public interface Repository<T> {
    T findById(long id);
}
"#;
    let (symbols, _) = extract_file("java", source, Arc::from("Repository.java"), true, 0);
    let ifaces: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Interface)
        .collect();
    assert_eq!(ifaces.len(), 1);
    assert_eq!(ifaces[0].name, "Repository");
    assert_eq!(ifaces[0].kind, SymbolKind::Interface);
}

#[test]
fn test_java_public_enum() {
    let source = r#"
public enum Status {
    ACTIVE,
    INACTIVE,
    PENDING
}
"#;
    let (symbols, _) = extract_file("java", source, Arc::from("Status.java"), true, 0);
    let enums: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Enum)
        .collect();
    assert_eq!(enums.len(), 1);
    assert_eq!(enums[0].name, "Status");
    assert_eq!(enums[0].kind, SymbolKind::Enum);
}

#[test]
fn test_java_public_method_with_params() {
    let source = r#"
public class OrderService {
    public Order processOrder(String customerId, int quantity) {
        return null;
    }
}
"#;
    let (symbols, _) = extract_file("java", source, Arc::from("OrderService.java"), true, 0);
    let methods: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .collect();
    assert_eq!(methods.len(), 1);
    let m = &methods[0];
    assert_eq!(m.name, "processOrder");
    assert_eq!(m.kind, SymbolKind::Method);
    assert_eq!(m.parent_symbol.as_deref(), Some("OrderService"));
    assert_eq!(m.return_type.as_deref(), Some("Order"));
    let params = m.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "customerId");
    assert_eq!(params[0].type_annotation.as_deref(), Some("String"));
    assert_eq!(params[1].name, "quantity");
    assert_eq!(params[1].type_annotation.as_deref(), Some("int"));
}

#[test]
fn test_java_static_method_as_function() {
    let source = r#"
public class MathUtils {
    public static double calculateArea(double radius) {
        return Math.PI * radius * radius;
    }
}
"#;
    let (symbols, _) = extract_file("java", source, Arc::from("MathUtils.java"), true, 0);
    let funcs: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Function)
        .collect();
    assert_eq!(funcs.len(), 1);
    assert_eq!(funcs[0].name, "calculateArea");
    assert_eq!(funcs[0].kind, SymbolKind::Function);
    assert_eq!(funcs[0].parent_symbol.as_deref(), Some("MathUtils"));
    assert_eq!(funcs[0].return_type.as_deref(), Some("double"));
}

#[test]
fn test_java_constant() {
    let source = r#"
public class AppConfig {
    public static final String API_VERSION = "v2";
    public static final int MAX_RETRIES = 3;
    private static final String SECRET = "hidden";
}
"#;
    let (symbols, _) = extract_file("java", source, Arc::from("AppConfig.java"), true, 0);
    let constants: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Constant)
        .collect();
    assert_eq!(constants.len(), 2);
    assert_eq!(constants[0].name, "API_VERSION");
    assert_eq!(constants[0].parent_symbol.as_deref(), Some("AppConfig"));
    assert_eq!(constants[1].name, "MAX_RETRIES");
}

#[test]
fn test_java_skip_private_class() {
    let source = r#"
private class InternalHelper {
    public void doSomething() {}
}
"#;
    let (symbols, _) = extract_file("java", source, Arc::from("InternalHelper.java"), true, 0);
    assert!(symbols.is_empty());
}

#[test]
fn test_java_skip_package_private_method() {
    let source = r#"
public class Service {
    void internalMethod(String data) {
    }

    private void secretMethod() {
    }

    public void publicMethod() {
    }
}
"#;
    let (symbols, _) = extract_file("java", source, Arc::from("Service.java"), true, 0);
    let methods: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method || s.kind == SymbolKind::Function)
        .collect();
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name, "publicMethod");
}

#[test]
fn test_java_call_references() {
    let source = r#"package com.example;

import java.util.List;

public class UserService {
    private Database db;

    public User fetchUser(String id) {
        return db.lookup(id);
    }

    public void saveUser(User u) {
        validate(u);
        db.insert(u);
    }

    private void validate(User u) {}
}
"#;
    let (_syms, refs) = extract_file("java", source, Arc::from("UserService.java"), false, 0);
    let mut call_refs: Vec<(String, usize, Option<String>)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| (r.name.clone(), r.line, r.enclosing_symbol.clone()))
        .collect();
    call_refs.sort();
    assert_eq!(
        call_refs,
        vec![
            ("insert".into(), 14, Some("UserService.saveUser".into())),
            ("lookup".into(), 9, Some("UserService.fetchUser".into())),
            ("validate".into(), 13, Some("UserService.saveUser".into())),
        ]
    );
}

#[test]
fn test_java_type_references() {
    let source = r#"class Repo {}
class Service {
  Repo load(Repo r) { return r; }
}
"#;
    let (_syms, refs) = extract_file("java", source, Arc::from("types.java"), false, 0);
    let mut type_refs: Vec<(String, usize)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Type)
        .map(|r| (r.name.clone(), r.line))
        .collect();
    type_refs.sort();
    assert_eq!(type_refs, vec![("Repo".into(), 3), ("Repo".into(), 3)]);
}

#[test]
fn test_java_impl_references() {
    let source = r#"package com.example;

public class ConcreteService extends BaseService implements Cacheable, Auditable {
}
"#;
    let (_syms, refs) = extract_file("java", source, Arc::from("CS.java"), false, 0);
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(impl_names.contains(&"BaseService"), "got {:?}", impl_names);
    assert!(impl_names.contains(&"Cacheable"), "got {:?}", impl_names);
    assert!(impl_names.contains(&"Auditable"), "got {:?}", impl_names);
}

#[test]
fn test_java_import_references() {
    let source = r#"package com.example;

import java.util.List;
import java.util.Map;
"#;
    let (_syms, refs) = extract_file("java", source, Arc::from("X.java"), false, 0);
    let imp_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    // Lookup by `symbol_references(name="List")` must match — capture the
    // simple name from `java.util.List`, not the full qualified path.
    assert!(imp_names.contains(&"List"), "got {:?}", imp_names);
    assert!(imp_names.contains(&"Map"), "got {:?}", imp_names);
}

#[test]
fn test_java_inner_class_call_enclosing_uses_inner_method() {
    let source = r#"class Outer {
  void outer() {
    class Inner {
      void run() { helper(); }
      void helper() {}
    }
  }
}
"#;
    let (_syms, refs) = extract_file("java", source, Arc::from("inner.java"), false, 0);
    let helper_call = refs
        .iter()
        .find(|r| r.kind == ReferenceKind::Call && r.name == "helper")
        .unwrap();
    assert_eq!(helper_call.line, 4);
    // Qualified through every enclosing method/class, innermost last, so the
    // call is unambiguously attributed to `Inner.run`, not to any other
    // same-named method elsewhere in the file.
    assert_eq!(
        helper_call.enclosing_symbol.as_deref(),
        Some("Outer.outer.Inner.run")
    );
}

// ============================================================
// Kotlin tests (ported from kotlin.rs)
// ============================================================

#[test]
fn test_kotlin_class() {
    let source = r#"class UserService {
    fun validate(token: String): Boolean {
        return true
    }
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("UserService.kt"), true, 0);
    let class_sym = symbols.iter().find(|s| s.name == "UserService").unwrap();
    assert_eq!(class_sym.kind, SymbolKind::Class);
    assert_eq!(class_sym.line, 1);
    assert!(
        class_sym
            .signature
            .as_ref()
            .unwrap()
            .contains("class UserService")
    );
}

#[test]
fn test_kotlin_interface() {
    let source = r#"interface Repository {
    fun findById(id: String): Entity?
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("Repository.kt"), true, 0);
    let iface = symbols
        .iter()
        .find(|s| s.name == "Repository")
        .expect("should find Repository");
    assert_eq!(iface.kind, SymbolKind::Interface);
    assert!(
        iface
            .signature
            .as_ref()
            .unwrap()
            .contains("interface Repository")
    );
}

#[test]
fn test_kotlin_object() {
    let source = r#"object DatabaseConfig {
    val url = "jdbc:postgresql://localhost/db"
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("Config.kt"), true, 0);
    let obj = symbols
        .iter()
        .find(|s| s.name == "DatabaseConfig")
        .expect("should find DatabaseConfig");
    assert_eq!(obj.kind, SymbolKind::Class);
    assert!(
        obj.signature
            .as_ref()
            .unwrap()
            .contains("object DatabaseConfig")
    );
}

#[test]
fn test_kotlin_enum_class() {
    let source = r#"enum class Status {
    ACTIVE,
    INACTIVE,
    SUSPENDED
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("Status.kt"), true, 0);
    let enum_sym = symbols
        .iter()
        .find(|s| s.name == "Status")
        .expect("should find Status");
    assert_eq!(enum_sym.kind, SymbolKind::Enum);
    assert!(
        enum_sym
            .signature
            .as_ref()
            .unwrap()
            .contains("enum class Status")
    );
}

#[test]
fn test_kotlin_top_level_function() {
    let source = r#"fun processPayment(amount: Double, currency: String): Receipt {
    return Receipt()
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("Payment.kt"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "processPayment");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(
        sym.signature
            .as_ref()
            .unwrap()
            .contains("fun processPayment")
    );
    assert_eq!(sym.return_type.as_deref(), Some("Receipt"));
    let params = sym.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "amount");
    assert_eq!(params[0].type_annotation.as_deref(), Some("Double"));
    assert_eq!(params[1].name, "currency");
    assert_eq!(params[1].type_annotation.as_deref(), Some("String"));
}

#[test]
fn test_kotlin_class_method() {
    let source = r#"class AuthService {
    fun validate(token: String): Boolean {
        return true
    }
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("AuthService.kt"), true, 0);
    let method = symbols
        .iter()
        .find(|s| s.name == "validate")
        .expect("should find validate method");
    assert_eq!(method.kind, SymbolKind::Method);
    assert_eq!(method.parent_symbol.as_deref(), Some("AuthService"));
    let params = method.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "token");
    assert_eq!(params[0].type_annotation.as_deref(), Some("String"));
    assert_eq!(method.return_type.as_deref(), Some("Boolean"));
}

#[test]
fn test_kotlin_skip_private_class() {
    let source = r#"private class InternalHelper {
    fun doSomething() {}
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("Internal.kt"), true, 0);
    assert!(
        symbols.is_empty(),
        "private class and its methods should be skipped"
    );
}

#[test]
fn test_kotlin_skip_internal_function() {
    let source = r#"internal fun helperFunction(x: Int): Int {
    return x * 2
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("Helper.kt"), true, 0);
    assert!(symbols.is_empty(), "internal function should be skipped");
}

#[test]
fn test_kotlin_skip_private_method() {
    let source = r#"class PublicService {
    fun publicMethod(): String {
        return ""
    }

    private fun secretMethod(): String {
        return ""
    }
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("Service.kt"), true, 0);
    assert!(symbols.iter().any(|s| s.name == "PublicService"));
    assert!(symbols.iter().any(|s| s.name == "publicMethod"));
    assert!(
        !symbols.iter().any(|s| s.name == "secretMethod"),
        "private method should be skipped"
    );
}

#[test]
fn test_kotlin_function_no_return_type() {
    let source = r#"fun doWork(task: String) {
    println(task)
}"#;
    let (symbols, _) = extract_file("kt", source, Arc::from("Work.kt"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "doWork");
    assert!(symbols[0].return_type.is_none());
}

// ============================================================
// Protobuf tests (ported from proto.rs)
// ============================================================

#[test]
fn test_proto_message() {
    let source = r#"syntax = "proto3";

message SearchRequest {
  string query = 1;
  int32 page_number = 2;
}
"#;
    let (symbols, _) = extract_file("proto", source, Arc::from("search.proto"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "SearchRequest");
    assert_eq!(sym.kind, SymbolKind::Struct);
    assert_eq!(sym.signature.as_deref(), Some("message SearchRequest"));
    assert_eq!(sym.line, 3);
}

#[test]
fn test_proto_service_and_rpc() {
    let source = r#"syntax = "proto3";

service SearchService {
  rpc Search (SearchRequest) returns (SearchResponse);
}
"#;
    let (symbols, _) = extract_file("proto", source, Arc::from("search.proto"), true, 0);
    assert_eq!(symbols.len(), 2);

    let svc = &symbols[0];
    assert_eq!(svc.name, "SearchService");
    assert_eq!(svc.kind, SymbolKind::Interface);
    assert_eq!(svc.signature.as_deref(), Some("service SearchService"));

    let rpc = &symbols[1];
    assert_eq!(rpc.name, "Search");
    assert_eq!(rpc.kind, SymbolKind::Method);
    assert_eq!(rpc.parent_symbol.as_deref(), Some("SearchService"));
    assert_eq!(rpc.return_type.as_deref(), Some("SearchResponse"));
    let params = rpc.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "request");
    assert_eq!(params[0].type_annotation.as_deref(), Some("SearchRequest"));
    assert_eq!(
        rpc.signature.as_deref(),
        Some("rpc Search(SearchRequest) returns (SearchResponse)")
    );
}

#[test]
fn test_proto_streaming_rpc() {
    let source = r#"syntax = "proto3";

service StreamService {
  rpc ClientStream (stream UpdateRequest) returns (UpdateResponse);
  rpc ServerStream (GetRequest) returns (stream GetResponse);
  rpc BiDiStream (stream ChatMessage) returns (stream ChatMessage);
}
"#;
    let (symbols, _) = extract_file("proto", source, Arc::from("stream.proto"), true, 0);
    assert_eq!(symbols.len(), 4);

    let client_rpc = &symbols[1];
    assert_eq!(client_rpc.name, "ClientStream");
    let params = client_rpc.parameters.as_ref().unwrap();
    assert_eq!(
        params[0].type_annotation.as_deref(),
        Some("stream UpdateRequest")
    );
    assert_eq!(client_rpc.return_type.as_deref(), Some("UpdateResponse"));

    let server_rpc = &symbols[2];
    assert_eq!(server_rpc.name, "ServerStream");
    let params = server_rpc.parameters.as_ref().unwrap();
    assert_eq!(params[0].type_annotation.as_deref(), Some("GetRequest"));
    assert_eq!(
        server_rpc.return_type.as_deref(),
        Some("stream GetResponse")
    );

    let bidi_rpc = &symbols[3];
    assert_eq!(bidi_rpc.name, "BiDiStream");
    let params = bidi_rpc.parameters.as_ref().unwrap();
    assert_eq!(
        params[0].type_annotation.as_deref(),
        Some("stream ChatMessage")
    );
    assert_eq!(bidi_rpc.return_type.as_deref(), Some("stream ChatMessage"));
}

#[test]
fn test_proto_enum() {
    let source = r#"syntax = "proto3";

enum Status {
  UNKNOWN = 0;
  ACTIVE = 1;
  INACTIVE = 2;
}
"#;
    let (symbols, _) = extract_file("proto", source, Arc::from("status.proto"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "Status");
    assert_eq!(sym.kind, SymbolKind::Enum);
    assert_eq!(sym.signature.as_deref(), Some("enum Status"));
}

#[test]
fn test_proto_nested_message_and_enum() {
    let source = r#"syntax = "proto3";

message Outer {
  string id = 1;

  message Inner {
    int32 value = 1;
  }

  enum Color {
    RED = 0;
    BLUE = 1;
  }
}
"#;
    let (symbols, _) = extract_file("proto", source, Arc::from("nested.proto"), true, 0);
    assert_eq!(symbols.len(), 3);

    let outer = &symbols[0];
    assert_eq!(outer.name, "Outer");
    assert_eq!(outer.kind, SymbolKind::Struct);
    assert!(outer.parent_symbol.is_none());

    let inner = &symbols[1];
    assert_eq!(inner.name, "Inner");
    assert_eq!(inner.kind, SymbolKind::Struct);
    assert_eq!(inner.parent_symbol.as_deref(), Some("Outer"));
    assert_eq!(inner.signature.as_deref(), Some("message Outer.Inner"));

    let color = &symbols[2];
    assert_eq!(color.name, "Color");
    assert_eq!(color.kind, SymbolKind::Enum);
    assert_eq!(color.parent_symbol.as_deref(), Some("Outer"));
    assert_eq!(color.signature.as_deref(), Some("enum Outer.Color"));
}

#[test]
fn test_proto_oneof() {
    let source = r#"syntax = "proto3";

message SampleMessage {
  oneof test_oneof {
    string name = 4;
    int32 id = 5;
  }
}
"#;
    let (symbols, _) = extract_file("proto", source, Arc::from("oneof.proto"), true, 0);
    assert_eq!(symbols.len(), 2);

    let msg = &symbols[0];
    assert_eq!(msg.name, "SampleMessage");

    let oneof = &symbols[1];
    assert_eq!(oneof.name, "test_oneof");
    assert_eq!(oneof.kind, SymbolKind::Type);
    assert_eq!(oneof.parent_symbol.as_deref(), Some("SampleMessage"));
    assert_eq!(
        oneof.signature.as_deref(),
        Some("oneof SampleMessage.test_oneof")
    );
}

#[test]
fn test_proto_empty_file() {
    let (symbols, _) = extract_file("proto", "", Arc::from("empty.proto"), true, 0);
    assert!(symbols.is_empty());
}

// ============================================================
// C tests
// ============================================================

#[test]
fn test_c_function() {
    let source = r#"int process_payment(float amount, const char *currency) {
    return 0;
}
"#;
    let (symbols, _) = extract_file("c", source, Arc::from("payment.c"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "process_payment");
    assert_eq!(sym.kind, SymbolKind::Function);
    let params = sym.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "amount");
    assert_eq!(params[1].name, "currency");
}

#[test]
fn test_c_struct() {
    let source = r#"struct AuthService {
    int id;
    char *name;
};
"#;
    let (symbols, _) = extract_file("c", source, Arc::from("auth.c"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "AuthService");
    assert_eq!(symbols[0].kind, SymbolKind::Struct);
}

#[test]
fn test_c_enum() {
    let source = r#"enum Status {
    ACTIVE,
    INACTIVE
};
"#;
    let (symbols, _) = extract_file("c", source, Arc::from("types.c"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Status");
    assert_eq!(symbols[0].kind, SymbolKind::Enum);
}

#[test]
fn test_c_skip_static() {
    let source = r#"static int internal_helper(void) {
    return 42;
}
"#;
    let (symbols, _) = extract_file("c", source, Arc::from("internal.c"), true, 0);
    assert!(symbols.is_empty());
}

#[test]
fn test_c_typedef() {
    let source = r#"typedef unsigned long size_t;
"#;
    let (symbols, _) = extract_file("c", source, Arc::from("types.c"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "size_t");
    assert_eq!(symbols[0].kind, SymbolKind::Type);
}

// ============================================================
// C++ tests
// ============================================================

#[test]
fn test_cpp_class() {
    let source = r#"class UserService {
public:
    void validate(std::string token) {}
private:
    int count;
};
"#;
    let (symbols, _) = extract_file("cpp", source, Arc::from("user_service.cpp"), true, 0);
    let classes: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].name, "UserService");
}

#[test]
fn test_cpp_struct() {
    let source = r#"struct Point {
    double x;
    double y;
};
"#;
    let (symbols, _) = extract_file("cpp", source, Arc::from("point.cpp"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Point" && s.kind == SymbolKind::Struct)
    );
}

#[test]
fn test_cpp_function() {
    let source = r#"int calculate(double a, double b) {
    return 0;
}
"#;
    let (symbols, _) = extract_file("cpp", source, Arc::from("math.cpp"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "calculate" && s.kind == SymbolKind::Function)
    );
}

#[test]
fn test_cpp_enum() {
    let source = r#"enum Color {
    RED,
    GREEN,
    BLUE
};
"#;
    let (symbols, _) = extract_file("cpp", source, Arc::from("colors.cpp"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Color" && s.kind == SymbolKind::Enum)
    );
}

#[test]
fn test_cpp_namespace() {
    let source = r#"namespace MyLib {
    class Widget {};
}
"#;
    let (symbols, _) = extract_file("cpp", source, Arc::from("widget.cpp"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "MyLib" && s.kind == SymbolKind::Class)
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Widget" && s.kind == SymbolKind::Class)
    );
}

// ============================================================
// C# tests
// ============================================================

#[test]
fn test_csharp_class() {
    let source = r#"
public class UserService {
    private int count;
}
"#;
    let (symbols, _) = extract_file("cs", source, Arc::from("UserService.cs"), true, 0);
    let classes: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Class)
        .collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].name, "UserService");
}

#[test]
fn test_csharp_interface() {
    let source = r#"
public interface IRepository {
    void Save(string data);
}
"#;
    let (symbols, _) = extract_file("cs", source, Arc::from("IRepository.cs"), true, 0);
    let ifaces: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Interface)
        .collect();
    assert_eq!(ifaces.len(), 1);
    assert_eq!(ifaces[0].name, "IRepository");
}

#[test]
fn test_csharp_struct() {
    let source = r#"
public struct Point {
    public double X;
    public double Y;
}
"#;
    let (symbols, _) = extract_file("cs", source, Arc::from("Point.cs"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Point" && s.kind == SymbolKind::Struct)
    );
}

#[test]
fn test_csharp_enum() {
    let source = r#"
public enum Status {
    Active,
    Inactive,
    Pending
}
"#;
    let (symbols, _) = extract_file("cs", source, Arc::from("Status.cs"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Status" && s.kind == SymbolKind::Enum)
    );
}

#[test]
fn test_csharp_method() {
    let source = r#"
public class OrderService {
    public Order ProcessOrder(string customerId, int quantity) {
        return null;
    }
}
"#;
    let (symbols, _) = extract_file("cs", source, Arc::from("OrderService.cs"), true, 0);
    let methods: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method)
        .collect();
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name, "ProcessOrder");
    assert_eq!(methods[0].parent_symbol.as_deref(), Some("OrderService"));
}

#[test]
fn test_csharp_skip_private() {
    let source = r#"
public class Service {
    private void SecretMethod() {}
    public void PublicMethod() {}
}
"#;
    let (symbols, _) = extract_file("cs", source, Arc::from("Service.cs"), true, 0);
    let methods: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method || s.kind == SymbolKind::Function)
        .collect();
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name, "PublicMethod");
}

// ============================================================
// Swift tests
// ============================================================

#[test]
fn test_swift_class() {
    let source = r#"public class AuthService {
    public func validate(token: String) -> Bool {
        return true
    }
}
"#;
    let (symbols, _) = extract_file("swift", source, Arc::from("AuthService.swift"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "AuthService" && s.kind == SymbolKind::Class)
    );
}

#[test]
fn test_swift_struct() {
    let source = r#"public struct Point {
    var x: Double
    var y: Double
}
"#;
    let (symbols, _) = extract_file("swift", source, Arc::from("Point.swift"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Point" && s.kind == SymbolKind::Struct)
    );
}

#[test]
fn test_swift_protocol() {
    let source = r#"public protocol Repository {
    func findById(id: String) -> Entity?
}
"#;
    let (symbols, _) = extract_file("swift", source, Arc::from("Repository.swift"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Repository" && s.kind == SymbolKind::Interface)
    );
}

#[test]
fn test_swift_enum() {
    let source = r#"public enum Status {
    case active
    case inactive
}
"#;
    let (symbols, _) = extract_file("swift", source, Arc::from("Status.swift"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Status" && s.kind == SymbolKind::Enum)
    );
}

#[test]
fn test_swift_function() {
    let source = r#"public func processPayment(amount: Double, currency: String) -> Receipt {
    return Receipt()
}
"#;
    let (symbols, _) = extract_file("swift", source, Arc::from("Payment.swift"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "processPayment");
    assert_eq!(symbols[0].kind, SymbolKind::Function);
}

#[test]
fn test_swift_skip_private() {
    let source = r#"private func internalHelper() -> Void {}
fileprivate func alsoPrivate() -> Void {}
"#;
    let (symbols, _) = extract_file("swift", source, Arc::from("Internal.swift"), true, 0);
    assert!(symbols.is_empty());
}

// ============================================================
// PHP tests
// ============================================================

#[test]
fn test_php_class() {
    let source = r#"<?php
class UserService {
    public function validate(string $token): bool {
        return true;
    }
}
"#;
    let (symbols, _) = extract_file("php", source, Arc::from("UserService.php"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "UserService" && s.kind == SymbolKind::Class)
    );
}

#[test]
fn test_php_interface() {
    let source = r#"<?php
interface Repository {
    public function findById(int $id): Entity;
}
"#;
    let (symbols, _) = extract_file("php", source, Arc::from("Repository.php"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Repository" && s.kind == SymbolKind::Interface)
    );
}

#[test]
fn test_php_function() {
    let source = r#"<?php
function process_payment(float $amount, string $currency): Receipt {
    return new Receipt();
}
"#;
    let (symbols, _) = extract_file("php", source, Arc::from("payment.php"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "process_payment" && s.kind == SymbolKind::Function)
    );
}

#[test]
fn test_php_trait() {
    let source = r#"<?php
trait Loggable {
    public function log(string $message): void {}
}
"#;
    let (symbols, _) = extract_file("php", source, Arc::from("Loggable.php"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Loggable" && s.kind == SymbolKind::Trait)
    );
}

#[test]
fn test_php_enum() {
    let source = r#"<?php
enum Status {
    case Active;
    case Inactive;
}
"#;
    let (symbols, _) = extract_file("php", source, Arc::from("Status.php"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Status" && s.kind == SymbolKind::Enum)
    );
}

// ============================================================
// Scala tests
// ============================================================

#[test]
fn test_scala_class() {
    let source = r#"class UserService {
  def validate(token: String): Boolean = true
}
"#;
    let (symbols, _) = extract_file("scala", source, Arc::from("UserService.scala"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "UserService" && s.kind == SymbolKind::Class)
    );
}

#[test]
fn test_scala_object() {
    let source = r#"object DatabaseConfig {
  val url = "jdbc:postgresql://localhost/db"
}
"#;
    let (symbols, _) = extract_file("scala", source, Arc::from("Config.scala"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "DatabaseConfig" && s.kind == SymbolKind::Class)
    );
}

#[test]
fn test_scala_trait() {
    let source = r#"trait Repository {
  def findById(id: String): Option[Entity]
}
"#;
    let (symbols, _) = extract_file("scala", source, Arc::from("Repository.scala"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "Repository" && s.kind == SymbolKind::Interface)
    );
}

#[test]
fn test_scala_function() {
    let source = r#"def processPayment(amount: Double, currency: String): Receipt = {
  Receipt()
}
"#;
    let (symbols, _) = extract_file("scala", source, Arc::from("Payment.scala"), true, 0);
    assert!(symbols.iter().any(|s| s.name == "processPayment"));
}

#[test]
fn test_scala_skip_private() {
    let source = r#"private def internalHelper(): Unit = {}
"#;
    let (symbols, _) = extract_file("scala", source, Arc::from("Internal.scala"), true, 0);
    assert!(symbols.is_empty());
}

#[test]
fn test_scala_call_references() {
    let source = r#"package com.example

object Service {
  def process(req: Request): Response = {
    val cfg = parseConfig(req.body)
    buildResponse(cfg)
  }

  def parseConfig(body: String): Config = Config(body)
  def buildResponse(cfg: Config): Response = Response(cfg.key)
}
"#;
    let (_syms, refs) = extract_file("scala", source, Arc::from("svc.scala"), false, 0);
    let mut call_refs: Vec<(String, usize, Option<String>)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| (r.name.clone(), r.line, r.enclosing_symbol.clone()))
        .collect();
    call_refs.sort();
    assert_eq!(
        call_refs,
        vec![
            ("Config".into(), 9, Some("Service.parseConfig".into())),
            ("Response".into(), 10, Some("Service.buildResponse".into())),
            ("buildResponse".into(), 6, Some("Service.process".into())),
            ("parseConfig".into(), 5, Some("Service.process".into())),
        ]
    );
}

#[test]
fn test_scala_type_references() {
    let source = r#"class Config
object Service {
  def load(c: Config): Config = c
}
"#;
    let (_syms, refs) = extract_file("scala", source, Arc::from("types.scala"), false, 0);
    let mut type_refs: Vec<(String, usize)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Type)
        .map(|r| (r.name.clone(), r.line))
        .collect();
    type_refs.sort();
    assert_eq!(type_refs, vec![("Config".into(), 3), ("Config".into(), 3)]);
}

#[test]
fn test_scala_import_references() {
    let source = r#"import Config

object Service
"#;
    let (_syms, refs) = extract_file("scala", source, Arc::from("imports.scala"), false, 0);
    let import_refs: Vec<(String, usize)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| (r.name.clone(), r.line))
        .collect();
    assert_eq!(import_refs, vec![("Config".into(), 1)]);
}

#[test]
fn test_scala_impl_references() {
    let source = r#"trait Service
trait Cacheable
class Base
class MyService extends Base with Service with Cacheable
"#;
    let (_syms, refs) = extract_file("scala", source, Arc::from("s.scala"), false, 0);
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(impl_names.contains(&"Base"), "got {:?}", impl_names);
    assert!(impl_names.contains(&"Service"));
    assert!(impl_names.contains(&"Cacheable"));
}

// ============================================================
// Zig tests
// ============================================================

#[test]
fn test_zig_function() {
    let source = r#"pub fn processPayment(amount: f64) !Receipt {
    return error.NotImplemented;
}
"#;
    let (symbols, _) = extract_file("zig", source, Arc::from("payment.zig"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "processPayment" && s.kind == SymbolKind::Function)
    );
}

#[test]
fn test_zig_skip_non_pub() {
    let source = r#"fn internalHelper() void {
    return;
}
"#;
    let (symbols, _) = extract_file("zig", source, Arc::from("internal.zig"), true, 0);
    assert!(symbols.is_empty());
}

#[test]
fn test_zig_const() {
    let source = r#"pub const MAX_SIZE: usize = 1024;
"#;
    let (symbols, _) = extract_file("zig", source, Arc::from("config.zig"), true, 0);
    assert!(symbols.iter().any(|s| s.name == "MAX_SIZE"));
}

// ============================================================
// Elixir tests (tree-sitter)
// ============================================================

#[test]
fn test_elixir_module() {
    let source = r#"defmodule MyApp.Users do
  def get_user(id) do
    Repo.get(User, id)
  end
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/users.ex"), true, 0);
    let module = symbols.iter().find(|s| s.name == "MyApp.Users").unwrap();
    assert_eq!(module.kind, SymbolKind::Class);
    assert_eq!(
        module.signature.as_deref(),
        Some("defmodule MyApp.Users do")
    );
    assert!(module.parent_symbol.is_none());

    let func = symbols.iter().find(|s| s.name == "get_user").unwrap();
    assert_eq!(func.kind, SymbolKind::Function);
    assert_eq!(func.signature.as_deref(), Some("def get_user(id)"));
    assert_eq!(func.parent_symbol.as_deref(), Some("MyApp.Users"));
    let params = func.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "id");
}

#[test]
fn test_elixir_protocol() {
    let source = r#"defprotocol MyApp.Serializable do
  @doc "Serializes the given value"
  def serialize(value)
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/serializable.ex"), true, 0);
    let proto = symbols
        .iter()
        .find(|s| s.name == "MyApp.Serializable")
        .unwrap();
    assert_eq!(proto.kind, SymbolKind::Interface);
    assert_eq!(
        proto.signature.as_deref(),
        Some("defprotocol MyApp.Serializable do")
    );

    let func = symbols.iter().find(|s| s.name == "serialize").unwrap();
    assert_eq!(func.kind, SymbolKind::Function);
    assert_eq!(func.parent_symbol.as_deref(), Some("MyApp.Serializable"));
}

#[test]
fn test_elixir_skip_private() {
    let source = r#"defmodule MyApp do
  def public_fn do
    :ok
  end

  defp private_fn do
    :secret
  end

  defmacrop private_macro(x), do: x
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/my_app.ex"), true, 0);
    assert!(symbols.iter().any(|s| s.name == "public_fn"));
    assert!(!symbols.iter().any(|s| s.name == "private_fn"));
    assert!(!symbols.iter().any(|s| s.name == "private_macro"));
}

#[test]
fn test_elixir_macro() {
    let source = r#"defmodule MyApp.Router do
  defmacro route(path, handler) do
    quote do
      @routes [{unquote(path), unquote(handler)} | @routes]
    end
  end
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/router.ex"), true, 0);
    let mac = symbols.iter().find(|s| s.name == "route").unwrap();
    assert_eq!(mac.kind, SymbolKind::Function);
    assert_eq!(
        mac.signature.as_deref(),
        Some("defmacro route(path, handler)")
    );
    assert_eq!(mac.parent_symbol.as_deref(), Some("MyApp.Router"));
    let params = mac.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "path");
    assert_eq!(params[1].name, "handler");
}

#[test]
fn test_elixir_callback() {
    let source = r#"defmodule MyApp.Behaviour do
  @callback init(opts :: keyword()) :: {:ok, state :: term()} | {:error, reason :: term()}
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/behaviour.ex"), true, 0);
    let cb = symbols.iter().find(|s| s.name == "init").unwrap();
    assert_eq!(cb.kind, SymbolKind::Method);
    assert!(
        cb.signature
            .as_deref()
            .unwrap()
            .starts_with("@callback init(")
    );
    assert_eq!(cb.parent_symbol.as_deref(), Some("MyApp.Behaviour"));
    assert!(cb.return_type.is_some());
}

#[test]
fn test_elixir_type() {
    let source = r#"defmodule MyApp.Types do
  @type name :: String.t()
  @type pair(a, b) :: {a, b}
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/types.ex"), true, 0);
    let name_type = symbols.iter().find(|s| s.name == "name").unwrap();
    assert_eq!(name_type.kind, SymbolKind::Type);
    assert_eq!(
        name_type.signature.as_deref(),
        Some("@type name :: String.t()")
    );
    assert_eq!(name_type.return_type.as_deref(), Some("String.t()"));

    let pair_type = symbols.iter().find(|s| s.name == "pair").unwrap();
    assert_eq!(pair_type.kind, SymbolKind::Type);
}

#[test]
fn test_elixir_nested_modules() {
    let source = r#"defmodule MyApp.Outer do
  defmodule Inner do
    def hello do
      :world
    end
  end
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/outer.ex"), true, 0);
    let outer = symbols.iter().find(|s| s.name == "MyApp.Outer").unwrap();
    assert!(outer.parent_symbol.is_none());

    let inner = symbols.iter().find(|s| s.name == "Inner").unwrap();
    assert_eq!(inner.parent_symbol.as_deref(), Some("MyApp.Outer"));

    let func = symbols.iter().find(|s| s.name == "hello").unwrap();
    assert_eq!(func.parent_symbol.as_deref(), Some("Inner"));
}

#[test]
fn test_elixir_function_with_defaults() {
    let source = r#"defmodule Config do
  def load(path, opts \\ []) do
    :ok
  end
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/config.ex"), true, 0);
    let func = symbols.iter().find(|s| s.name == "load").unwrap();
    let params = func.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "path");
    assert_eq!(params[1].name, "opts");
}

#[test]
fn test_elixir_function_with_guard() {
    let source = r#"defmodule Math do
  def abs(x) when x >= 0 do
    x
  end
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/math.ex"), true, 0);
    let func = symbols.iter().find(|s| s.name == "abs").unwrap();
    assert_eq!(func.kind, SymbolKind::Function);
    assert_eq!(func.signature.as_deref(), Some("def abs(x)"));
}

#[test]
fn test_elixir_one_line_function() {
    let source = r#"defmodule MyModule do
  def greet(name), do: "Hello, #{name}"
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/my_module.ex"), true, 0);
    let func = symbols.iter().find(|s| s.name == "greet").unwrap();
    assert_eq!(func.kind, SymbolKind::Function);
    assert_eq!(func.parent_symbol.as_deref(), Some("MyModule"));
}

#[test]
fn test_elixir_empty_source() {
    let (symbols, _) = extract_file("ex", "", Arc::from("empty.ex"), true, 0);
    assert!(symbols.is_empty());
}

#[test]
fn test_elixir_comments_ignored() {
    let source = r#"defmodule Foo do
  # def not_a_function do
  #   :nope
  # end

  def real_function do
    :ok
  end
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/foo.ex"), true, 0);
    assert!(!symbols.iter().any(|s| s.name == "not_a_function"));
    assert!(symbols.iter().any(|s| s.name == "real_function"));
}

#[test]
fn test_elixir_dotted_module_name() {
    let source = "defmodule MyApp.Web.Controllers.UserController do\nend\n";
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/user_controller.ex"), true, 0);
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "MyApp.Web.Controllers.UserController");
    assert_eq!(symbols[0].kind, SymbolKind::Class);
}

#[test]
fn test_elixir_exs_extension() {
    let source = r#"defmodule MyApp.MixProject do
  def project do
    [app: :my_app]
  end
end
"#;
    let (symbols, _) = extract_file("exs", source, Arc::from("mix.exs"), true, 0);
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "MyApp.MixProject" && s.kind == SymbolKind::Class)
    );
    assert!(
        symbols
            .iter()
            .any(|s| s.name == "project" && s.kind == SymbolKind::Function)
    );
}

#[test]
fn test_elixir_defdelegate() {
    let source = r#"defmodule MyApp.Facade do
  defdelegate greet(name), to: MyApp.Greeter
  defdelegate version, to: MyApp.Config
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/facade.ex"), true, 0);
    let greet = symbols.iter().find(|s| s.name == "greet").unwrap();
    assert_eq!(greet.kind, SymbolKind::Function);
    assert_eq!(greet.signature.as_deref(), Some("defdelegate greet(name)"));
    assert_eq!(greet.parent_symbol.as_deref(), Some("MyApp.Facade"));
    let params = greet.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "name");

    let version = symbols.iter().find(|s| s.name == "version").unwrap();
    assert_eq!(version.kind, SymbolKind::Function);
    assert_eq!(version.signature.as_deref(), Some("defdelegate version"));
}

#[test]
fn test_elixir_opaque_type() {
    let source = r#"defmodule MyApp.Token do
  @opaque t :: %__MODULE__{value: String.t()}
end
"#;
    let (symbols, _) = extract_file("ex", source, Arc::from("lib/token.ex"), true, 0);
    let opaque = symbols.iter().find(|s| s.name == "t").unwrap();
    assert_eq!(opaque.kind, SymbolKind::Type);
    assert!(
        opaque
            .signature
            .as_deref()
            .unwrap()
            .starts_with("@opaque t ::")
    );
    assert!(opaque.return_type.is_some());
    assert_eq!(opaque.parent_symbol.as_deref(), Some("MyApp.Token"));
}

// ============================================================
// Haskell tests
// ============================================================

#[test]
fn test_haskell_function_with_type_signature() {
    let source = r#"
process :: Int -> String -> Bool
process x y = length (show x) > length y
"#;
    let (symbols, _) = extract_file("hs", source, Arc::from("Lib.hs"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "process");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert_eq!(
        sym.signature.as_deref(),
        Some("process :: Int -> String -> Bool")
    );
    assert_eq!(sym.return_type.as_deref(), Some("Bool"));
    let params = sym.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "x");
    assert_eq!(params[1].name, "y");
}

#[test]
fn test_haskell_function_without_type_signature() {
    let source = r#"
greet name = "Hello " ++ name
"#;
    let (symbols, _) = extract_file("hs", source, Arc::from("Lib.hs"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "greet");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(sym.return_type.is_none());
    let params = sym.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name, "name");
}

#[test]
fn test_haskell_data_type() {
    let source = r#"
data Maybe a = Nothing | Just a
"#;
    let (symbols, _) = extract_file("hs", source, Arc::from("Data.hs"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "Maybe");
    assert_eq!(sym.kind, SymbolKind::Struct);
}

#[test]
fn test_haskell_newtype() {
    let source = r#"
newtype Wrapper a = Wrapper a
"#;
    let (symbols, _) = extract_file("hs", source, Arc::from("Types.hs"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "Wrapper");
    assert_eq!(sym.kind, SymbolKind::Struct);
}

#[test]
fn test_haskell_type_alias() {
    let source = r#"
type Name = String
"#;
    let (symbols, _) = extract_file("hs", source, Arc::from("Types.hs"), true, 0);
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "Name");
    assert_eq!(sym.kind, SymbolKind::Type);
}

#[test]
fn test_haskell_type_class_with_methods() {
    let source = r#"
class Printable a where
    display :: a -> String
    preview :: a -> Int -> String
"#;
    let (symbols, _) = extract_file("hs", source, Arc::from("Class.hs"), true, 0);
    // class + 2 method signatures
    assert_eq!(symbols.len(), 3);

    assert_eq!(symbols[0].name, "Printable");
    assert_eq!(symbols[0].kind, SymbolKind::Trait);

    assert_eq!(symbols[1].name, "display");
    assert_eq!(symbols[1].kind, SymbolKind::Method);
    assert_eq!(symbols[1].parent_symbol.as_deref(), Some("Printable"));

    assert_eq!(symbols[2].name, "preview");
    assert_eq!(symbols[2].kind, SymbolKind::Method);
    assert_eq!(symbols[2].parent_symbol.as_deref(), Some("Printable"));
}

#[test]
fn test_haskell_multiple_declarations() {
    let source = r#"
data Color = Red | Green | Blue

type Palette = [Color]

fromString :: String -> Maybe Color
fromString "red" = Just Red
fromString "green" = Just Green
fromString "blue" = Just Blue
fromString _ = Nothing
"#;
    let (symbols, _) = extract_file("hs", source, Arc::from("Color.hs"), true, 0);
    assert_eq!(symbols.len(), 3);

    assert_eq!(symbols[0].name, "Color");
    assert_eq!(symbols[0].kind, SymbolKind::Struct);

    assert_eq!(symbols[1].name, "Palette");
    assert_eq!(symbols[1].kind, SymbolKind::Type);

    assert_eq!(symbols[2].name, "fromString");
    assert_eq!(symbols[2].kind, SymbolKind::Function);
    assert_eq!(symbols[2].return_type.as_deref(), Some("Maybe Color"));
}

// ============================================================
// Perl tests
// ============================================================

#[test]
fn test_perl_package_declaration() {
    let source = "package Foo::Bar;\n\nuse strict;\n";
    let (symbols, _) = extract_file("pm", source, Arc::from("lib/Foo/Bar.pm"), true, 0);

    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Foo::Bar");
    assert_eq!(symbols[0].kind, SymbolKind::Class);
    assert_eq!(symbols[0].signature.as_deref(), Some("package Foo::Bar"));
    assert_eq!(symbols[0].line, 1);
}

#[test]
fn test_perl_top_level_sub() {
    let source = r#"sub greet {
    my ($name) = @_;
    print "Hello, $name\n";
}
"#;
    let (symbols, _) = extract_file("pl", source, Arc::from("script.pl"), true, 0);

    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "greet");
    assert_eq!(symbols[0].kind, SymbolKind::Function);
    assert_eq!(symbols[0].signature.as_deref(), Some("sub greet"));
    assert!(symbols[0].parent_symbol.is_none());
}

#[test]
fn test_perl_method_inside_package() {
    let source = r#"package MyApp::Auth;

sub new {
    my ($class, %args) = @_;
    return bless \%args, $class;
}

sub validate {
    my ($self, $token) = @_;
    return 1;
}
"#;
    let (symbols, _) = extract_file("pm", source, Arc::from("lib/MyApp/Auth.pm"), true, 0);

    assert_eq!(symbols.len(), 3);

    assert_eq!(symbols[0].name, "MyApp::Auth");
    assert_eq!(symbols[0].kind, SymbolKind::Class);

    assert_eq!(symbols[1].name, "new");
    assert_eq!(symbols[1].kind, SymbolKind::Method);
    assert_eq!(symbols[1].parent_symbol.as_deref(), Some("MyApp::Auth"));
    assert_eq!(
        symbols[1].signature.as_deref(),
        Some("sub MyApp::Auth::new")
    );

    assert_eq!(symbols[2].name, "validate");
    assert_eq!(symbols[2].kind, SymbolKind::Method);
    assert_eq!(symbols[2].parent_symbol.as_deref(), Some("MyApp::Auth"));
}

#[test]
fn test_perl_skip_private_subs() {
    let source = r#"package Foo;

sub public_method {
    return 1;
}

sub _private_helper {
    return 2;
}
"#;
    let (symbols, _) = extract_file("pm", source, Arc::from("lib/Foo.pm"), true, 0);

    assert_eq!(symbols.len(), 2); // package + public_method only
    assert_eq!(symbols[0].name, "Foo");
    assert_eq!(symbols[1].name, "public_method");
}

#[test]
fn test_perl_multiple_packages() {
    let source = r#"package First::Package;

sub alpha {
    return 1;
}

package Second::Package;

sub beta {
    return 2;
}
"#;
    let (symbols, _) = extract_file("pm", source, Arc::from("lib/Multi.pm"), true, 0);

    assert_eq!(symbols.len(), 4);

    assert_eq!(symbols[0].name, "First::Package");
    assert_eq!(symbols[0].kind, SymbolKind::Class);

    assert_eq!(symbols[1].name, "alpha");
    assert_eq!(symbols[1].kind, SymbolKind::Method);
    assert_eq!(symbols[1].parent_symbol.as_deref(), Some("First::Package"));

    assert_eq!(symbols[2].name, "Second::Package");
    assert_eq!(symbols[2].kind, SymbolKind::Class);

    assert_eq!(symbols[3].name, "beta");
    assert_eq!(symbols[3].kind, SymbolKind::Method);
    assert_eq!(symbols[3].parent_symbol.as_deref(), Some("Second::Package"));
}

#[test]
fn test_perl_empty_source() {
    let (symbols, _) = extract_file("pl", "", Arc::from("empty.pl"), true, 0);
    assert!(symbols.is_empty());
}

#[test]
fn test_perl_line_numbers() {
    let source = r#"package Foo;

sub bar {
    return 1;
}

sub baz {
    return 2;
}
"#;
    let (symbols, _) = extract_file("pm", source, Arc::from("lib/Foo.pm"), true, 0);

    assert_eq!(symbols.len(), 3);
    assert_eq!(symbols[0].line, 1); // package Foo
    assert_eq!(symbols[1].line, 3); // sub bar
    assert_eq!(symbols[2].line, 7); // sub baz
}

#[test]
fn test_perl_call_references() {
    let source = r#"package My::Service;

use strict;

sub load {
    my ($path) = @_;
    my $raw = read_file($path);
    return parse_raw($raw);
}

sub parse_raw { my ($r) = @_; return {}; }

1;
"#;
    let (_syms, refs) = extract_file("pm", source, Arc::from("Service.pm"), false, 0);
    let names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| r.name.as_str())
        .collect();
    assert!(names.contains(&"read_file"), "got {:?}", names);
    assert!(names.contains(&"parse_raw"));
}

#[test]
fn test_perl_use_references() {
    let source = r#"package Main;
use strict;
use My::Utils;
use JSON::PP;
1;
"#;
    let (_syms, refs) = extract_file("pm", source, Arc::from("Main.pm"), false, 0);
    let imp_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.as_str())
        .collect();
    // Capture module names. `strict` is in stoplist — skip.
    assert!(
        imp_names.iter().any(|n| n.contains("Utils")),
        "got {:?}",
        imp_names
    );
    assert!(imp_names.iter().any(|n| n.contains("JSON")));
}

// ============================================================
// Ruby tests
// ============================================================

#[test]
fn test_ruby_class() {
    let source = r#"class UserService
  def initialize(db)
    @db = db
  end
end
"#;
    let (symbols, _) = extract_file(
        "rb",
        source,
        Arc::from("app/services/user_service.rb"),
        true,
        0,
    );

    let class_sym = symbols.iter().find(|s| s.name == "UserService").unwrap();
    assert_eq!(class_sym.kind, SymbolKind::Class);
    assert_eq!(class_sym.signature.as_deref(), Some("class UserService"));
    assert_eq!(class_sym.line, 1);
}

#[test]
fn test_ruby_module() {
    let source = r#"module Authentication
  def authenticate(token)
    verify(token)
  end
end
"#;
    let (symbols, _) = extract_file(
        "rb",
        source,
        Arc::from("app/concerns/authentication.rb"),
        true,
        0,
    );

    let mod_sym = symbols.iter().find(|s| s.name == "Authentication").unwrap();
    assert_eq!(mod_sym.kind, SymbolKind::Class);
    assert_eq!(mod_sym.signature.as_deref(), Some("module Authentication"));
}

#[test]
fn test_ruby_instance_method() {
    let source = r#"class OrderProcessor
  def process(order_id, amount)
    # process logic
  end
end
"#;
    let (symbols, _) = extract_file(
        "rb",
        source,
        Arc::from("app/services/order_processor.rb"),
        true,
        0,
    );

    let method_sym = symbols.iter().find(|s| s.name == "process").unwrap();
    assert_eq!(method_sym.kind, SymbolKind::Method);
    assert_eq!(method_sym.parent_symbol.as_deref(), Some("OrderProcessor"));
    assert_eq!(
        method_sym.signature.as_deref(),
        Some("def process(order_id, amount)")
    );

    let params = method_sym.parameters.as_ref().unwrap();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].name, "order_id");
    assert_eq!(params[1].name, "amount");
}

#[test]
fn test_ruby_class_method() {
    let source = r#"class Config
  def self.load(path)
    new(path)
  end
end
"#;
    let (symbols, _) = extract_file("rb", source, Arc::from("lib/config.rb"), true, 0);

    let method_sym = symbols.iter().find(|s| s.name == "load").unwrap();
    assert_eq!(method_sym.kind, SymbolKind::Function);
    assert_eq!(method_sym.parent_symbol.as_deref(), Some("Config"));
    assert_eq!(method_sym.signature.as_deref(), Some("def self.load(path)"));
}

#[test]
fn test_ruby_top_level_function() {
    let source = r#"def main
  puts "hello"
end
"#;
    let (symbols, _) = extract_file("rb", source, Arc::from("script.rb"), true, 0);

    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "main");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(sym.parent_symbol.is_none());
    assert_eq!(sym.signature.as_deref(), Some("def main"));
}

#[test]
fn test_ruby_inheritance() {
    let source = r#"class AdminController < ApplicationController
  def index
    render :index
  end
end
"#;
    let (symbols, _) = extract_file(
        "rb",
        source,
        Arc::from("app/controllers/admin_controller.rb"),
        true,
        0,
    );

    let class_sym = symbols
        .iter()
        .find(|s| s.name == "AdminController")
        .unwrap();
    assert_eq!(class_sym.kind, SymbolKind::Class);
    assert_eq!(
        class_sym.signature.as_deref(),
        Some("class AdminController < ApplicationController")
    );

    let method_sym = symbols.iter().find(|s| s.name == "index").unwrap();
    assert_eq!(method_sym.kind, SymbolKind::Method);
    assert_eq!(method_sym.parent_symbol.as_deref(), Some("AdminController"));
}

#[test]
fn test_ruby_nested_class_in_module() {
    let source = r#"module Payments
  class Processor
    def charge(amount)
      # charge logic
    end
  end
end
"#;
    let (symbols, _) = extract_file(
        "rb",
        source,
        Arc::from("lib/payments/processor.rb"),
        true,
        0,
    );

    let mod_sym = symbols.iter().find(|s| s.name == "Payments").unwrap();
    assert_eq!(mod_sym.kind, SymbolKind::Class);

    let class_sym = symbols.iter().find(|s| s.name == "Processor").unwrap();
    assert_eq!(class_sym.kind, SymbolKind::Class);
    assert_eq!(class_sym.parent_symbol.as_deref(), Some("Payments"));

    let method_sym = symbols.iter().find(|s| s.name == "charge").unwrap();
    assert_eq!(method_sym.kind, SymbolKind::Method);
    assert_eq!(method_sym.parent_symbol.as_deref(), Some("Processor"));
}

#[test]
fn test_ruby_method_with_special_params() {
    let source = r#"def create(name, *args, **opts, &block)
  # ...
end
"#;
    let (symbols, _) = extract_file("rb", source, Arc::from("factory.rb"), true, 0);

    assert_eq!(symbols.len(), 1);
    let params = symbols[0].parameters.as_ref().unwrap();
    assert_eq!(params.len(), 4);
    assert_eq!(params[0].name, "name");
    assert!(params[0].type_annotation.is_none());
    assert_eq!(params[1].name, "args");
    assert_eq!(params[1].type_annotation.as_deref(), Some("*"));
    assert_eq!(params[2].name, "opts");
    assert_eq!(params[2].type_annotation.as_deref(), Some("**"));
    assert_eq!(params[3].name, "block");
    assert_eq!(params[3].type_annotation.as_deref(), Some("&"));
}

#[test]
fn test_ruby_empty_source() {
    let (symbols, _) = extract_file("rb", "", Arc::from("empty.rb"), true, 0);
    assert!(symbols.is_empty());
}

#[test]
fn test_ruby_call_references() {
    let source = r#"require 'json'

class Loader
  def load(path)
    raw = File.read(path)
    JSON.parse(raw)
  end

  def save(path, data)
    File.write(path, JSON.dump(data))
  end
end
"#;
    let (_syms, refs) = extract_file("rb", source, Arc::from("loader.rb"), false, 0);
    let mut call_refs: Vec<(String, usize, Option<String>)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .map(|r| (r.name.clone(), r.line, r.enclosing_symbol.clone()))
        .collect();
    call_refs.sort();
    assert_eq!(
        call_refs,
        vec![
            ("dump".into(), 10, Some("Loader.save".into())),
            ("parse".into(), 6, Some("Loader.load".into())),
            ("read".into(), 5, Some("Loader.load".into())),
            ("write".into(), 10, Some("Loader.save".into())),
        ]
    );
}

#[test]
fn test_ruby_type_references() {
    let source = r#"class Loader
  def load(raw)
    JSON.parse(raw)
  end
end
"#;
    let (_syms, refs) = extract_file("rb", source, Arc::from("types.rb"), false, 0);
    let type_refs: Vec<(String, usize)> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Type)
        .map(|r| (r.name.clone(), r.line))
        .collect();
    assert_eq!(type_refs, vec![("JSON".into(), 3)]);
}

#[test]
fn test_ruby_impl_references() {
    let source = r#"class Derived < Base
  include Comparable
  include Enumerable
  extend ModuleMethods
end
"#;
    let (_syms, refs) = extract_file("rb", source, Arc::from("d.rb"), false, 0);
    let impl_names: Vec<&str> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Impl)
        .map(|r| r.name.as_str())
        .collect();
    assert!(
        impl_names.contains(&"Base"),
        "expected superclass Base, got {:?}",
        impl_names
    );
    assert!(impl_names.contains(&"Comparable"));
    assert!(impl_names.contains(&"Enumerable"));
    assert!(impl_names.contains(&"ModuleMethods"));
}

#[test]
fn test_ruby_require_references() {
    let source = r#"require 'json'
require_relative './util'
"#;
    let (_syms, refs) = extract_file("rb", source, Arc::from("m.rb"), false, 0);
    let mut imp_names: Vec<String> = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .map(|r| r.name.clone())
        .collect();
    imp_names.sort();
    assert_eq!(imp_names, vec!["./util".to_string(), "json".to_string()]);
}

#[test]
fn test_ruby_block_call_uses_outer_method_as_enclosing() {
    let source = r#"class Job
  def run(items)
    items.each do |item|
      process(item)
    end
  end
end
"#;
    let (_syms, refs) = extract_file("rb", source, Arc::from("job.rb"), false, 0);
    let process_call = refs
        .iter()
        .find(|r| r.kind == ReferenceKind::Call && r.name == "process")
        .unwrap();
    assert_eq!(process_call.line, 4);
    assert_eq!(process_call.enclosing_symbol.as_deref(), Some("Job.run"));
}

#[test]
fn test_ruby_type_call_dedup_for_constant_call() {
    let source = r#"def load(raw)
  JSON(raw)
end
"#;
    let (_syms, refs) = extract_file("rb", source, Arc::from("dedup.rb"), false, 0);
    let json_calls: Vec<&ReferenceInfo> = refs
        .iter()
        .filter(|r| r.name == "JSON" && r.kind == ReferenceKind::Call)
        .collect();
    let json_types: Vec<&ReferenceInfo> = refs
        .iter()
        .filter(|r| r.name == "JSON" && r.kind == ReferenceKind::Type)
        .collect();
    assert_eq!(json_calls.len(), 1, "JSON(raw) should emit one call-ref");
    assert_eq!(
        json_calls[0].enclosing_symbol.as_deref(),
        Some("load"),
        "JSON call should be attributed to load"
    );
    assert!(
        json_types.is_empty(),
        "type-ref at the same node/line must be deduplicated against call-ref"
    );
}

#[test]
fn test_type_definition_not_double_captured_as_ref() {
    // The type declaration "type Config struct" should NOT produce a self-reference
    // via @reference.type — only the uses in parameters/fields should.
    let source = r#"package main

type Config struct { Key string }

func use(c Config) Config { return c }
"#;
    let (syms, refs) = extract_file("go", source, Arc::from("main.go"), false, 0);

    // Config should be a defined struct
    assert!(
        syms.iter()
            .any(|s| s.name == "Config" && s.kind == SymbolKind::Struct)
    );

    // Config should appear as type-ref at the parameter and return positions (line 5),
    // but NOT at the declaration site (line 3).
    let config_type_refs: Vec<&ReferenceInfo> = refs
        .iter()
        .filter(|r| r.name == "Config" && r.kind == ReferenceKind::Type)
        .collect();

    // At least one type-ref (parameter/return), but none at line 3 (the definition)
    assert!(
        !config_type_refs.is_empty(),
        "Config should have at least one type reference (param/return)"
    );
    for r in &config_type_refs {
        assert_ne!(
            r.line, 3,
            "Config's own definition at line 3 should not produce a self-reference, got {:?}",
            r
        );
    }
}

#[test]
fn test_impl_refs_dedup_type_refs_at_same_node() {
    // Java: `extends BaseService implements Cacheable` — the Base/Cacheable
    // identifiers should each produce ONLY one Impl ref, not a redundant
    // Type ref at the same node.
    let source = r#"package com.example;
public class MyService extends BaseService implements Cacheable {
}
"#;
    let (_syms, refs) = extract_file("java", source, Arc::from("MS.java"), false, 0);

    for target in ["BaseService", "Cacheable"] {
        let hits: Vec<&ReferenceInfo> = refs.iter().filter(|r| r.name == target).collect();
        let kinds: Vec<_> = hits.iter().map(|r| r.kind).collect();
        assert!(
            kinds.contains(&ReferenceKind::Impl),
            "expected Impl ref for {}, got {:?}",
            target,
            kinds
        );
        assert!(
            !kinds.contains(&ReferenceKind::Type),
            "{} should NOT also be recorded as Type (duplicate at impl site), got {:?}",
            target,
            kinds
        );
    }

    // Scala: extends Base with Trait — same dedup invariant
    let scala_src = r#"trait Service
class Base
class MyService extends Base with Service
"#;
    let (_syms, refs) = extract_file("scala", scala_src, Arc::from("s.scala"), false, 0);
    let base_hits: Vec<_> = refs.iter().filter(|r| r.name == "Base").collect();
    let base_kinds: Vec<_> = base_hits.iter().map(|r| r.kind).collect();
    assert!(
        base_kinds.contains(&ReferenceKind::Impl),
        "Scala: expected Impl for Base, got {:?}",
        base_kinds
    );
    assert!(
        !base_kinds.contains(&ReferenceKind::Type),
        "Scala: Base should NOT also be Type at impl site, got {:?}",
        base_kinds
    );
}

#[test]
fn test_no_self_reference_at_definition_site_across_languages() {
    // Cross-language invariant: for every tier-1 language with reference
    // extraction, a definition's own name node at its declaration site must
    // never be emitted as a reference record. Uses elsewhere in the file are
    // expected — only the definition line should be ref-free for that name.
    //
    // This guards against per-language regressions where a new @reference.*
    // capture accidentally matches the definition's name node.
    struct Case {
        ext: &'static str,
        source: &'static str,
        def_name: &'static str,
        def_line: usize,
    }

    let cases = [
        // Go: type_identifier at the declaration AND at use sites.
        Case {
            ext: "go",
            source: "package main\ntype Config struct { Key string }\nfunc use(c Config) Config { return c }\n",
            def_name: "Config",
            def_line: 2,
        },
        // Python: identifier at class_definition name AND in type annotations.
        Case {
            ext: "py",
            source: "class Config:\n    pass\n\ndef use(c: Config) -> Config:\n    return c\n",
            def_name: "Config",
            def_line: 1,
        },
        // Java: identifier at class_declaration name, type_identifier at use sites.
        Case {
            ext: "java",
            source: "public class Config {}\nclass Svc { public Config use(Config c) { return c; } }\n",
            def_name: "Config",
            def_line: 1,
        },
        // TypeScript: type_identifier at interface_declaration name AND at
        // use sites. Non-exported to exercise the suppression-only patterns
        // that seed def_name_ranges for unexported type-like declarations.
        Case {
            ext: "ts",
            source: "interface Config { key: string }\nfunction use(c: Config): Config { return c }\n",
            def_name: "Config",
            def_line: 1,
        },
        // JavaScript: identifier at class_declaration name, referenced via extends (impl).
        Case {
            ext: "js",
            source: "export class Config {}\nexport class Sub extends Config {}\n",
            def_name: "Config",
            def_line: 1,
        },
        // Perl: package declaration name, referenced via `use` later.
        Case {
            ext: "pl",
            source: "package Config;\n\npackage Main;\nuse Config;\n",
            def_name: "Config",
            def_line: 1,
        },
        // Ruby: constant at class name AND as a type-ref constant in body.
        Case {
            ext: "rb",
            source: "class Config\nend\n\nclass Svc\n  def use\n    Config.new\n  end\nend\n",
            def_name: "Config",
            def_line: 1,
        },
        // Scala: type_identifier at class name AND at use sites.
        Case {
            ext: "scala",
            source: "class Config\nclass Svc { def use(c: Config): Config = c }\n",
            def_name: "Config",
            def_line: 1,
        },
    ];

    for case in cases {
        let (_, refs) = extract_file(case.ext, case.source, Arc::from("f"), false, 0);
        for r in &refs {
            if r.name == case.def_name {
                assert_ne!(
                    r.line, case.def_line,
                    "[{}] '{}' defined at line {} should not produce a self-reference, got ref {:?}",
                    case.ext, case.def_name, case.def_line, r
                );
            }
        }
    }
}

#[test]
fn test_references_capped_at_limit() {
    // Python source that generates multiple call references
    let source = r#"
def main():
    foo()
    bar()
    baz()
    qux()
    quux()
"#;
    // Uncapped (0) — should get all references
    let (_, refs_uncapped) = extract_file("py", source, Arc::from("cap.py"), false, 0);
    let call_count = refs_uncapped
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .count();
    assert!(
        call_count >= 5,
        "expected at least 5 call refs uncapped, got {call_count}"
    );

    // Capped at 2 — should get at most 2 references total (before dedup filtering)
    let (_, refs_capped) = extract_file("py", source, Arc::from("cap.py"), false, 2);
    assert!(
        refs_capped.len() <= 2,
        "expected at most 2 refs with cap=2, got {}",
        refs_capped.len()
    );
}

#[test]
fn test_references_cap_zero_means_unlimited() {
    let source = r#"
def main():
    foo()
    bar()
    baz()
"#;
    let (_, refs) = extract_file("py", source, Arc::from("uncap.py"), false, 0);
    let call_count = refs
        .iter()
        .filter(|r| r.kind == ReferenceKind::Call)
        .count();
    assert!(
        call_count >= 3,
        "cap=0 should be unlimited, got {call_count} call refs"
    );
}
