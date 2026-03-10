use super::*;

// ============================================================
// Python tests (ported from python.rs)
// ============================================================

#[test]
fn test_python_function_with_type_hints() {
    let source = r#"def process_payment(amount: float, currency: str) -> Receipt:
    pass
"#;
    let symbols = extract_file("py", source, "pay.py");
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
    let symbols = extract_file("py", source, "auth.py");
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
fn test_python_function_no_hints() {
    let source = r#"def greet(name):
    return f"Hello {name}"
"#;
    let symbols = extract_file("py", source, "greet.py");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "greet");
    assert!(symbols[0].return_type.is_none());
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
    let symbols = extract_file("go", source, "handler.go");
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
    let symbols = extract_file("go", source, "auth.go");
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
    let symbols = extract_file("go", source, "handler.go");
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
    let symbols = extract_file("go", source, "auth.go");
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
    let symbols = extract_file("go", source, "internal.go");
    assert!(symbols.is_empty());
}

// ============================================================
// Rust tests (ported from rust_lang.rs)
// ============================================================

#[test]
fn test_rust_pub_function() {
    let source = r#"pub fn process_payment(amount: f64, currency: &str) -> Result<Receipt> {
    todo!()
}"#;
    let symbols = extract_file("rs", source, "src/pay.rs");
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "process_payment");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(sym.signature.as_ref().unwrap().contains("pub fn process_payment"));
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
    let symbols = extract_file("rs", source, "src/auth.rs");
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
    let symbols = extract_file("rs", source, "src/types.rs");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Status");
    assert_eq!(symbols[0].kind, SymbolKind::Enum);
}

#[test]
fn test_rust_pub_trait() {
    let source = r#"pub trait Handler {
    fn handle(&self) -> Result<()>;
}"#;
    let symbols = extract_file("rs", source, "src/handler.rs");
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
    let symbols = extract_file("rs", source, "src/auth.rs");
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
    let symbols = extract_file("rs", source, "src/internal.rs");
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
    let symbols = extract_file("ts", source, "src/pay.ts");
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
    let symbols = extract_file("ts", source, "src/auth.ts");
    assert!(symbols.len() >= 2);
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
    let symbols = extract_file("ts", source, "src/types.ts");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "UserConfig");
    assert_eq!(symbols[0].kind, SymbolKind::Interface);
}

#[test]
fn test_ts_exported_type_alias() {
    let source = "export type Result<T> = Success<T> | Failure;";
    let symbols = extract_file("ts", source, "src/types.ts");
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
    let symbols = extract_file("ts", source, "src/types.ts");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Status");
    assert_eq!(symbols[0].kind, SymbolKind::Enum);
}

#[test]
fn test_ts_exported_const() {
    let source = "export const MAX_RETRIES = 3;";
    let symbols = extract_file("ts", source, "src/config.ts");
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
    let symbols = extract_file("ts", source, "src/internal.ts");
    assert!(symbols.is_empty());
}

#[test]
fn test_ts_default_export_function() {
    let source = r#"export default function handler(req: Request): Response {
    return new Response();
}"#;
    let symbols = extract_file("ts", source, "src/handler.ts");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "handler");
    assert_eq!(symbols[0].kind, SymbolKind::Function);
}

#[test]
fn test_js_function() {
    let source = r#"export function greet(name) {
    return 'Hello ' + name;
}"#;
    let symbols = extract_file("js", source, "src/greet.js");
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "greet");
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
    let symbols = extract_file("java", source, "UserService.java");
    let classes: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Class).collect();
    assert_eq!(classes.len(), 1);
    assert_eq!(classes[0].name, "UserService");
    assert!(classes[0].signature.as_ref().unwrap().contains("class"));
    assert!(classes[0].signature.as_ref().unwrap().contains("UserService"));
}

#[test]
fn test_java_public_interface() {
    let source = r#"
public interface Repository<T> {
    T findById(long id);
}
"#;
    let symbols = extract_file("java", source, "Repository.java");
    let ifaces: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Interface).collect();
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
    let symbols = extract_file("java", source, "Status.java");
    let enums: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Enum).collect();
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
    let symbols = extract_file("java", source, "OrderService.java");
    let methods: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Method).collect();
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
    let symbols = extract_file("java", source, "MathUtils.java");
    let funcs: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Function).collect();
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
    let symbols = extract_file("java", source, "AppConfig.java");
    let constants: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Constant).collect();
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
    let symbols = extract_file("java", source, "InternalHelper.java");
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
    let symbols = extract_file("java", source, "Service.java");
    let methods: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::Method || s.kind == SymbolKind::Function)
        .collect();
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].name, "publicMethod");
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
    let symbols = extract_file("kt", source, "UserService.kt");
    let class_sym = symbols.iter().find(|s| s.name == "UserService").unwrap();
    assert_eq!(class_sym.kind, SymbolKind::Class);
    assert_eq!(class_sym.line, 1);
    assert!(class_sym.signature.as_ref().unwrap().contains("class UserService"));
}

#[test]
fn test_kotlin_interface() {
    let source = r#"interface Repository {
    fun findById(id: String): Entity?
}"#;
    let symbols = extract_file("kt", source, "Repository.kt");
    let iface = symbols.iter().find(|s| s.name == "Repository").expect("should find Repository");
    assert_eq!(iface.kind, SymbolKind::Interface);
    assert!(iface.signature.as_ref().unwrap().contains("interface Repository"));
}

#[test]
fn test_kotlin_object() {
    let source = r#"object DatabaseConfig {
    val url = "jdbc:postgresql://localhost/db"
}"#;
    let symbols = extract_file("kt", source, "Config.kt");
    let obj = symbols.iter().find(|s| s.name == "DatabaseConfig").expect("should find DatabaseConfig");
    assert_eq!(obj.kind, SymbolKind::Class);
    assert!(obj.signature.as_ref().unwrap().contains("object DatabaseConfig"));
}

#[test]
fn test_kotlin_enum_class() {
    let source = r#"enum class Status {
    ACTIVE,
    INACTIVE,
    SUSPENDED
}"#;
    let symbols = extract_file("kt", source, "Status.kt");
    let enum_sym = symbols.iter().find(|s| s.name == "Status").expect("should find Status");
    assert_eq!(enum_sym.kind, SymbolKind::Enum);
    assert!(enum_sym.signature.as_ref().unwrap().contains("enum class Status"));
}

#[test]
fn test_kotlin_top_level_function() {
    let source = r#"fun processPayment(amount: Double, currency: String): Receipt {
    return Receipt()
}"#;
    let symbols = extract_file("kt", source, "Payment.kt");
    assert_eq!(symbols.len(), 1);
    let sym = &symbols[0];
    assert_eq!(sym.name, "processPayment");
    assert_eq!(sym.kind, SymbolKind::Function);
    assert!(sym.signature.as_ref().unwrap().contains("fun processPayment"));
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
    let symbols = extract_file("kt", source, "AuthService.kt");
    let method = symbols.iter().find(|s| s.name == "validate").expect("should find validate method");
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
    let symbols = extract_file("kt", source, "Internal.kt");
    assert!(symbols.is_empty(), "private class and its methods should be skipped");
}

#[test]
fn test_kotlin_skip_internal_function() {
    let source = r#"internal fun helperFunction(x: Int): Int {
    return x * 2
}"#;
    let symbols = extract_file("kt", source, "Helper.kt");
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
    let symbols = extract_file("kt", source, "Service.kt");
    assert!(symbols.iter().any(|s| s.name == "PublicService"));
    assert!(symbols.iter().any(|s| s.name == "publicMethod"));
    assert!(!symbols.iter().any(|s| s.name == "secretMethod"), "private method should be skipped");
}

#[test]
fn test_kotlin_function_no_return_type() {
    let source = r#"fun doWork(task: String) {
    println(task)
}"#;
    let symbols = extract_file("kt", source, "Work.kt");
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
    let symbols = extract_file("proto", source, "search.proto");
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
    let symbols = extract_file("proto", source, "search.proto");
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
    let symbols = extract_file("proto", source, "stream.proto");
    assert_eq!(symbols.len(), 4);

    let client_rpc = &symbols[1];
    assert_eq!(client_rpc.name, "ClientStream");
    let params = client_rpc.parameters.as_ref().unwrap();
    assert_eq!(params[0].type_annotation.as_deref(), Some("stream UpdateRequest"));
    assert_eq!(client_rpc.return_type.as_deref(), Some("UpdateResponse"));

    let server_rpc = &symbols[2];
    assert_eq!(server_rpc.name, "ServerStream");
    let params = server_rpc.parameters.as_ref().unwrap();
    assert_eq!(params[0].type_annotation.as_deref(), Some("GetRequest"));
    assert_eq!(server_rpc.return_type.as_deref(), Some("stream GetResponse"));

    let bidi_rpc = &symbols[3];
    assert_eq!(bidi_rpc.name, "BiDiStream");
    let params = bidi_rpc.parameters.as_ref().unwrap();
    assert_eq!(params[0].type_annotation.as_deref(), Some("stream ChatMessage"));
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
    let symbols = extract_file("proto", source, "status.proto");
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
    let symbols = extract_file("proto", source, "nested.proto");
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
    let symbols = extract_file("proto", source, "oneof.proto");
    assert_eq!(symbols.len(), 2);

    let msg = &symbols[0];
    assert_eq!(msg.name, "SampleMessage");

    let oneof = &symbols[1];
    assert_eq!(oneof.name, "test_oneof");
    assert_eq!(oneof.kind, SymbolKind::Type);
    assert_eq!(oneof.parent_symbol.as_deref(), Some("SampleMessage"));
    assert_eq!(oneof.signature.as_deref(), Some("oneof SampleMessage.test_oneof"));
}

#[test]
fn test_proto_empty_file() {
    let symbols = extract_file("proto", "", "empty.proto");
    assert!(symbols.is_empty());
}
