use std::sync::{Arc, LazyLock};

use super::{SymbolInfo, SymbolKind};
use regex::Regex;

static PROGRAM_ID_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*PROGRAM-ID\.\s+([\w-]+)").unwrap());
static DIVISION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*(IDENTIFICATION|ENVIRONMENT|DATA|PROCEDURE)\s+DIVISION").unwrap());
static SECTION_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*([\w][\w-]*)\s+SECTION\s*\.").unwrap());
static COPY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*COPY\s+([\w-]+)").unwrap());
static FD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*FD\s+([\w-]+)").unwrap());
static LEVEL_01_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*01\s+([\w-]+)").unwrap());
static PARAGRAPH_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^\s*([A-Z0-9][\w-]*)\s*\.\s*$").unwrap());

/// Extract symbols from COBOL source code using regex-based parsing.
///
/// Extracts PROGRAM-ID as Class, sections and paragraphs in the PROCEDURE
/// DIVISION as Function/Method, COPY statements as Type, FD entries as Struct,
/// and 01-level data items as Constant.
///
/// COBOL is case-insensitive; all regexes use `(?i)`.
/// Comments (lines with `*` or `/` in column 7, or `*>` anywhere) are skipped.
pub fn extract(source: &str, file_path: Arc<str>) -> Vec<SymbolInfo> {
    let program_id_re = &*PROGRAM_ID_RE;
    let division_re = &*DIVISION_RE;
    let section_re = &*SECTION_RE;
    let copy_re = &*COPY_RE;
    let fd_re = &*FD_RE;
    let level_01_re = &*LEVEL_01_RE;
    let paragraph_re = &*PARAGRAPH_RE;

    let mut symbols = Vec::new();
    let mut current_division: Option<String> = None;
    let mut current_section: Option<String> = None;

    for (line_idx, line) in source.lines().enumerate() {
        let line_number = line_idx + 1;

        // Skip empty lines
        if line.trim().is_empty() {
            continue;
        }

        // Skip COBOL comments:
        // - Fixed-format: column 7 (index 6) is '*' or '/'
        // - Free-format: line starts with '*>' (possibly with leading whitespace)
        if line.len() > 6 && (line.as_bytes()[6] == b'*' || line.as_bytes()[6] == b'/') {
            continue;
        }
        if line.trim().starts_with("*>") {
            continue;
        }

        // Track current division
        if let Some(caps) = division_re.captures(line) {
            current_division = Some(caps[1].to_uppercase());
            current_section = None;
            continue;
        }

        // PROGRAM-ID (usually in IDENTIFICATION DIVISION)
        if let Some(caps) = program_id_re.captures(line) {
            let name = caps[1].trim_end_matches('.').to_string();
            let signature = format!("PROGRAM-ID. {}", name);

            symbols.push(SymbolInfo {
                name,
                kind: SymbolKind::Class,
                signature: Some(signature),
                file_path: file_path.clone(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: None,
                return_type: None,
                parameters: None,
            });
            continue;
        }

        // COPY statements (can appear in any division)
        if let Some(caps) = copy_re.captures(line) {
            let name = caps[1].trim_end_matches('.').to_string();
            let signature = format!("COPY {}", name);

            symbols.push(SymbolInfo {
                name,
                kind: SymbolKind::Type,
                signature: Some(signature),
                file_path: file_path.clone(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: None,
                return_type: None,
                parameters: None,
            });
            continue;
        }

        // FD entries (DATA DIVISION)
        if let Some(caps) = fd_re.captures(line) {
            let name = caps[1].trim_end_matches('.').to_string();
            let signature = format!("FD {}", name);

            symbols.push(SymbolInfo {
                name,
                kind: SymbolKind::Struct,
                signature: Some(signature),
                file_path: file_path.clone(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: None,
                return_type: None,
                parameters: None,
            });
            continue;
        }

        // 01-level data items (DATA DIVISION)
        if let Some(caps) = level_01_re.captures(line) {
            let name = caps[1].trim_end_matches('.').to_string();
            // Skip FILLER entries
            if name.eq_ignore_ascii_case("FILLER") {
                continue;
            }
            let signature = format!("01 {}", name);

            symbols.push(SymbolInfo {
                name,
                kind: SymbolKind::Constant,
                signature: Some(signature),
                file_path: file_path.clone(),
                line: line_number,
                visibility: "public".to_string(),
                parent_symbol: None,
                return_type: None,
                parameters: None,
            });
            continue;
        }

        // SECTION in PROCEDURE DIVISION
        let in_procedure = current_division.as_deref() == Some("PROCEDURE");
        if in_procedure {
            if let Some(caps) = section_re.captures(line) {
                let name = caps[1].to_string();
                let signature = format!("{} SECTION", name);

                symbols.push(SymbolInfo {
                    name: name.clone(),
                    kind: SymbolKind::Function,
                    signature: Some(signature),
                    file_path: file_path.clone(),
                    line: line_number,
                    visibility: "public".to_string(),
                    parent_symbol: None,
                    return_type: None,
                    parameters: None,
                });

                current_section = Some(name);
                continue;
            }

            // Paragraph names in PROCEDURE DIVISION
            if let Some(caps) = paragraph_re.captures(line) {
                let name = caps[1].to_string();

                // Skip COBOL reserved words that look like paragraphs
                let upper = name.to_uppercase();
                if matches!(
                    upper.as_str(),
                    "STOP" | "EXIT" | "PERFORM" | "CALL" | "GO" | "IF" | "ELSE"
                    | "END" | "MOVE" | "ADD" | "SUBTRACT" | "MULTIPLY" | "DIVIDE"
                    | "COMPUTE" | "DISPLAY" | "ACCEPT" | "READ" | "WRITE" | "OPEN"
                    | "CLOSE" | "RETURN" | "EVALUATE" | "WHEN" | "NOT" | "SET"
                    | "INITIALIZE" | "STRING" | "UNSTRING" | "INSPECT" | "SEARCH"
                    | "ALTER" | "CONTINUE" | "GOBACK" | "DELETE" | "SORT" | "MERGE"
                    | "REWRITE" | "START" | "GENERATE" | "TERMINATE" | "RELEASE"
                    | "REPLACE" | "EXEC"
                ) {
                    continue;
                }

                symbols.push(SymbolInfo {
                    name,
                    kind: SymbolKind::Method,
                    signature: None,
                    file_path: file_path.clone(),
                    line: line_number,
                    visibility: "public".to_string(),
                    parent_symbol: current_section.clone(),
                    return_type: None,
                    parameters: None,
                });
            }
        }
    }

    symbols
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn test_extract_program_id() {
        let source = "\
       IDENTIFICATION DIVISION.
       PROGRAM-ID. HELLO-WORLD.
       ";
        let symbols = extract(source, Arc::from("HELLO.cob"));

        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "HELLO-WORLD");
        assert_eq!(symbols[0].kind, SymbolKind::Class);
        assert_eq!(
            symbols[0].signature.as_deref(),
            Some("PROGRAM-ID. HELLO-WORLD")
        );
    }

    #[test]
    fn test_extract_sections() {
        let source = "\
       IDENTIFICATION DIVISION.
       PROGRAM-ID. TEST-PROG.
       PROCEDURE DIVISION.
       INIT-SECTION SECTION.
           DISPLAY \"HELLO\".
       PROCESS-SECTION SECTION.
           DISPLAY \"WORLD\".
       ";
        let symbols = extract(source, Arc::from("test.cob"));

        let sections: Vec<&SymbolInfo> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].name, "INIT-SECTION");
        assert_eq!(
            sections[0].signature.as_deref(),
            Some("INIT-SECTION SECTION")
        );
        assert_eq!(sections[1].name, "PROCESS-SECTION");
    }

    #[test]
    fn test_extract_paragraphs() {
        let source = "\
       IDENTIFICATION DIVISION.
       PROGRAM-ID. TEST-PROG.
       PROCEDURE DIVISION.
       MAIN-SECTION SECTION.
       INIT-PARA.
           DISPLAY \"INIT\".
       PROCESS-PARA.
           DISPLAY \"PROCESS\".
       ";
        let symbols = extract(source, Arc::from("test.cob"));

        let paragraphs: Vec<&SymbolInfo> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(paragraphs.len(), 2);
        assert_eq!(paragraphs[0].name, "INIT-PARA");
        assert_eq!(
            paragraphs[0].parent_symbol.as_deref(),
            Some("MAIN-SECTION")
        );
        assert_eq!(paragraphs[1].name, "PROCESS-PARA");
        assert_eq!(
            paragraphs[1].parent_symbol.as_deref(),
            Some("MAIN-SECTION")
        );
    }

    #[test]
    fn test_extract_copy() {
        let source = "\
       IDENTIFICATION DIVISION.
       PROGRAM-ID. TEST-PROG.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       COPY CUSTOMER-REC.
       COPY ACCOUNT-REC.
       ";
        let symbols = extract(source, Arc::from("test.cob"));

        let copies: Vec<&SymbolInfo> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Type)
            .collect();
        assert_eq!(copies.len(), 2);
        assert_eq!(copies[0].name, "CUSTOMER-REC");
        assert_eq!(copies[0].signature.as_deref(), Some("COPY CUSTOMER-REC"));
        assert_eq!(copies[1].name, "ACCOUNT-REC");
    }

    #[test]
    fn test_extract_fd() {
        let source = "\
       DATA DIVISION.
       FILE SECTION.
       FD CUSTOMER-FILE.
       01 CUSTOMER-RECORD.
          05 CUST-ID PIC 9(5).
       FD ACCOUNT-FILE.
       ";
        let symbols = extract(source, Arc::from("test.cob"));

        let fds: Vec<&SymbolInfo> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert_eq!(fds.len(), 2);
        assert_eq!(fds[0].name, "CUSTOMER-FILE");
        assert_eq!(fds[0].signature.as_deref(), Some("FD CUSTOMER-FILE"));
        assert_eq!(fds[1].name, "ACCOUNT-FILE");
    }

    #[test]
    fn test_extract_01_level() {
        let source = "\
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 WS-CUSTOMER-NAME PIC X(30).
       01 WS-ACCOUNT-BALANCE PIC 9(10)V99.
       01 FILLER PIC X(10).
       ";
        let symbols = extract(source, Arc::from("test.cob"));

        let data_items: Vec<&SymbolInfo> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();
        // FILLER should be skipped
        assert_eq!(data_items.len(), 2);
        assert_eq!(data_items[0].name, "WS-CUSTOMER-NAME");
        assert_eq!(data_items[0].signature.as_deref(), Some("01 WS-CUSTOMER-NAME"));
        assert_eq!(data_items[1].name, "WS-ACCOUNT-BALANCE");
    }

    #[test]
    fn test_case_insensitive() {
        let source = "\
       identification division.
       program-id. my-program.
       data division.
       working-storage section.
       copy my-copybook.
       01 ws-field pic x(10).
       procedure division.
       main-section section.
       main-para.
           display \"hello\".
       ";
        let symbols = extract(source, Arc::from("test.cbl"));

        assert!(symbols.iter().any(|s| s.name == "my-program" && s.kind == SymbolKind::Class));
        assert!(symbols.iter().any(|s| s.name == "my-copybook" && s.kind == SymbolKind::Type));
        assert!(symbols.iter().any(|s| s.name == "ws-field" && s.kind == SymbolKind::Constant));
        assert!(symbols.iter().any(|s| s.name == "main-section" && s.kind == SymbolKind::Function));
        assert!(symbols.iter().any(|s| s.name == "main-para" && s.kind == SymbolKind::Method));
    }

    #[test]
    fn test_paragraphs_only_in_procedure_division() {
        let source = "\
       IDENTIFICATION DIVISION.
       PROGRAM-ID. TEST-PROG.
       ENVIRONMENT DIVISION.
       CONFIGURATION SECTION.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       PROCEDURE DIVISION.
       REAL-PARA.
           DISPLAY \"HELLO\".
       ";
        let symbols = extract(source, Arc::from("test.cob"));

        let paragraphs: Vec<&SymbolInfo> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        // Only REAL-PARA should be extracted (in PROCEDURE DIVISION)
        assert_eq!(paragraphs.len(), 1);
        assert_eq!(paragraphs[0].name, "REAL-PARA");
    }

    #[test]
    fn test_skip_comments() {
        let source = "\
       IDENTIFICATION DIVISION.
      *THIS IS A COMMENT
       PROGRAM-ID. TEST-PROG.
      /THIS IS ALSO A COMMENT
       PROCEDURE DIVISION.
      *> Free-format comment
       REAL-PARA.
           DISPLAY \"HELLO\".
       ";
        let symbols = extract(source, Arc::from("test.cob"));

        assert_eq!(symbols.len(), 2); // PROGRAM-ID + REAL-PARA
        assert_eq!(symbols[0].name, "TEST-PROG");
        assert_eq!(symbols[1].name, "REAL-PARA");
    }

    #[test]
    fn test_empty_source() {
        let symbols = extract("", Arc::from("empty.cob"));
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_line_numbers() {
        let source = "\
       IDENTIFICATION DIVISION.
       PROGRAM-ID. TEST-PROG.
       PROCEDURE DIVISION.
       MAIN-PARA.
           DISPLAY \"HELLO\".
       ";
        let symbols = extract(source, Arc::from("test.cob"));

        assert_eq!(symbols[0].name, "TEST-PROG");
        assert_eq!(symbols[0].line, 2);
        assert_eq!(symbols[1].name, "MAIN-PARA");
        assert_eq!(symbols[1].line, 4);
    }

    #[test]
    fn test_full_cobol_program() {
        let source = "\
       IDENTIFICATION DIVISION.
       PROGRAM-ID. PAYROLL-CALC.
       ENVIRONMENT DIVISION.
       INPUT-OUTPUT SECTION.
       FILE-CONTROL.
           SELECT EMPLOYEE-FILE ASSIGN TO \"EMP.DAT\".
       DATA DIVISION.
       FILE SECTION.
       FD EMPLOYEE-FILE.
       01 EMPLOYEE-RECORD.
          05 EMP-ID PIC 9(5).
          05 EMP-NAME PIC X(30).
          05 EMP-SALARY PIC 9(7)V99.
       WORKING-STORAGE SECTION.
       01 WS-TOTAL-PAY PIC 9(10)V99.
       COPY PAYROLL-CONSTANTS.
       PROCEDURE DIVISION.
       MAIN-SECTION SECTION.
       START-PARA.
           PERFORM INIT-PARA.
           PERFORM CALC-PARA.
           STOP RUN.
       INIT-PARA.
           OPEN INPUT EMPLOYEE-FILE.
           INITIALIZE WS-TOTAL-PAY.
       CALC-SECTION SECTION.
       CALC-PARA.
           READ EMPLOYEE-FILE.
           ADD EMP-SALARY TO WS-TOTAL-PAY.
       ";
        let symbols = extract(source, Arc::from("PAYROLL.cob"));

        // PROGRAM-ID
        assert!(symbols.iter().any(|s| s.name == "PAYROLL-CALC" && s.kind == SymbolKind::Class));
        // FD
        assert!(symbols.iter().any(|s| s.name == "EMPLOYEE-FILE" && s.kind == SymbolKind::Struct));
        // 01-level
        assert!(symbols.iter().any(|s| s.name == "EMPLOYEE-RECORD" && s.kind == SymbolKind::Constant));
        assert!(symbols.iter().any(|s| s.name == "WS-TOTAL-PAY" && s.kind == SymbolKind::Constant));
        // COPY
        assert!(symbols.iter().any(|s| s.name == "PAYROLL-CONSTANTS" && s.kind == SymbolKind::Type));
        // Sections
        assert!(symbols.iter().any(|s| s.name == "MAIN-SECTION" && s.kind == SymbolKind::Function));
        assert!(symbols.iter().any(|s| s.name == "CALC-SECTION" && s.kind == SymbolKind::Function));
        // Paragraphs
        assert!(symbols.iter().any(|s| s.name == "START-PARA" && s.kind == SymbolKind::Method));
        assert!(symbols.iter().any(|s| s.name == "INIT-PARA" && s.kind == SymbolKind::Method));
        assert!(symbols.iter().any(|s| s.name == "CALC-PARA" && s.kind == SymbolKind::Method));

        // Verify parent_symbol for paragraphs
        let start = symbols.iter().find(|s| s.name == "START-PARA").unwrap();
        assert_eq!(start.parent_symbol.as_deref(), Some("MAIN-SECTION"));
        let calc = symbols.iter().find(|s| s.name == "CALC-PARA").unwrap();
        assert_eq!(calc.parent_symbol.as_deref(), Some("CALC-SECTION"));
    }

    #[test]
    fn test_numeric_prefixed_paragraphs() {
        let source = "\
       IDENTIFICATION DIVISION.
       PROGRAM-ID. TEST-PROG.
       PROCEDURE DIVISION.
       1000-INIT.
           DISPLAY \"INITIALIZING\".
       2000-PROCESS.
           DISPLAY \"PROCESSING\".
       9999-EXIT.
           STOP RUN.
       ";
        let symbols = extract(source, Arc::from("test.cob"));

        let paragraphs: Vec<&SymbolInfo> = symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(paragraphs.len(), 3);
        assert_eq!(paragraphs[0].name, "1000-INIT");
        assert_eq!(paragraphs[1].name, "2000-PROCESS");
        assert_eq!(paragraphs[2].name, "9999-EXIT");
    }
}
