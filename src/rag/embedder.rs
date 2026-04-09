use anyhow::Result;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::config::RagConfig;

pub struct Embedder {
    model: TextEmbedding,
}

impl Embedder {
    pub fn new(config: &RagConfig) -> Result<Self> {
        let model_name = config.model.as_deref().unwrap_or("BAAI/bge-small-en-v1.5");

        let model_enum = match model_name {
            "BAAI/bge-small-en-v1.5" => EmbeddingModel::BGESmallENV15,
            other => {
                anyhow::bail!(
                    "Unsupported embedding model: {other}. \
                     Only BAAI/bge-small-en-v1.5 is currently supported."
                );
            }
        };

        let mut options = InitOptions::new(model_enum).with_show_download_progress(true);

        if let Some(ref cache_dir) = config.cache_dir {
            let expanded = shellexpand::full(cache_dir)?.into_owned();
            options = options.with_cache_dir(std::path::PathBuf::from(expanded));
        }

        let model = TextEmbedding::try_new(options)?;
        Ok(Self { model })
    }

    pub fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.model.embed(texts, None)
    }
}

pub struct SymbolForEmbedding {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub signature: Option<String>,
    pub package: String,
    pub file_path: String,
}

pub fn symbol_to_text(sym: &SymbolForEmbedding) -> String {
    match &sym.signature {
        Some(sig) => format!(
            "{} {} in {} — {} @ {}",
            sym.kind, sym.name, sym.package, sig, sym.file_path
        ),
        None => format!(
            "{} {} in {} @ {}",
            sym.kind, sym.name, sym.package, sym.file_path
        ),
    }
}

pub fn embed_symbols(
    embedder: &Embedder,
    symbols: &[SymbolForEmbedding],
) -> Result<Vec<(i64, Vec<f32>)>> {
    if symbols.is_empty() {
        return Ok(Vec::new());
    }

    let texts: Vec<String> = symbols.iter().map(symbol_to_text).collect();
    let embeddings = embedder.embed(texts)?;

    let result: Vec<(i64, Vec<f32>)> = symbols
        .iter()
        .zip(embeddings)
        .map(|(sym, emb)| (sym.id, emb))
        .collect();

    Ok(result)
}

pub struct FileSymbol {
    pub name: String,
    pub kind: String,
    pub signature: Option<String>,
}

pub struct FileForEmbedding {
    pub id: i64,
    pub file_path: String,
    pub package: String,
    pub symbols: Vec<FileSymbol>,
}

pub fn file_to_text(file: &FileForEmbedding) -> String {
    if file.symbols.is_empty() {
        return format!("file {} in {}", file.file_path, file.package);
    }
    let mut sorted: Vec<&FileSymbol> = file.symbols.iter().collect();
    sorted.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));

    let prefix = format!("{} in {} — ", file.file_path, file.package);
    let budget = 1800usize.saturating_sub(prefix.len());
    let mut parts = Vec::new();
    let mut used = 0;
    for sym in &sorted {
        let part = match &sym.signature {
            Some(sig) if !sig.is_empty() => sig.clone(),
            _ => format!("{} {}", sym.kind, sym.name),
        };
        let cost = if parts.is_empty() {
            part.len()
        } else {
            part.len() + 2
        };
        if used + cost > budget {
            break;
        }
        used += cost;
        parts.push(part);
    }
    format!("{prefix}{}", parts.join(", "))
}

pub fn embed_files(
    embedder: &Embedder,
    files: &[FileForEmbedding],
    on_batch: impl Fn(usize),
) -> Result<Vec<(i64, Vec<f32>)>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }

    const BATCH_SIZE: usize = 64;
    let mut result = Vec::with_capacity(files.len());

    for chunk in files.chunks(BATCH_SIZE) {
        let texts: Vec<String> = chunk.iter().map(file_to_text).collect();
        let embeddings = embedder.embed(texts)?;
        for (file, emb) in chunk.iter().zip(embeddings) {
            result.push((file.id, emb));
        }
        on_batch(chunk.len());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_to_text_with_signature() {
        let sym = SymbolForEmbedding {
            id: 1,
            name: "validate".into(),
            kind: "method".into(),
            signature: Some("validate(token: string): Promise<boolean>".into()),
            package: "auth-service".into(),
            file_path: "services/auth/src/auth.ts".into(),
        };
        let text = symbol_to_text(&sym);
        assert_eq!(
            text,
            "method validate in auth-service — validate(token: string): Promise<boolean> @ services/auth/src/auth.ts"
        );
    }

    fn make_file(symbols: Vec<FileSymbol>) -> FileForEmbedding {
        FileForEmbedding {
            id: 1,
            file_path: "src/auth/middleware.rs".into(),
            package: "auth-service".into(),
            symbols,
        }
    }

    fn sym(name: &str, kind: &str, signature: Option<&str>) -> FileSymbol {
        FileSymbol {
            name: name.into(),
            kind: kind.into(),
            signature: signature.map(|s| s.into()),
        }
    }

    #[test]
    fn test_file_to_text_prefers_signature() {
        let file = make_file(vec![sym(
            "authenticate",
            "function",
            Some("fn authenticate(token: &str) -> Result<User>"),
        )]);
        let text = file_to_text(&file);
        assert_eq!(
            text,
            "src/auth/middleware.rs in auth-service — fn authenticate(token: &str) -> Result<User>"
        );
    }

    #[test]
    fn test_file_to_text_falls_back_without_signature() {
        let file = make_file(vec![sym("UserConfig", "struct", None)]);
        let text = file_to_text(&file);
        assert_eq!(
            text,
            "src/auth/middleware.rs in auth-service — struct UserConfig"
        );
    }

    #[test]
    fn test_file_to_text_empty_signature_treated_as_absent() {
        let file = make_file(vec![sym("process", "function", Some(""))]);
        let text = file_to_text(&file);
        assert_eq!(
            text,
            "src/auth/middleware.rs in auth-service — function process"
        );
    }

    #[test]
    fn test_file_to_text_empty_symbols() {
        let file = make_file(vec![]);
        let text = file_to_text(&file);
        assert_eq!(text, "file src/auth/middleware.rs in auth-service");
    }

    #[test]
    fn test_file_to_text_respects_budget() {
        let symbols: Vec<FileSymbol> = (0..200)
            .map(|i| {
                sym(
                    &format!("symbol_{i}"),
                    "function",
                    Some(&format!("fn symbol_{i}(x: i32) -> i32")),
                )
            })
            .collect();
        let file = make_file(symbols);
        let text = file_to_text(&file);
        assert!(
            text.len() <= 1800,
            "text length {} exceeds 1800",
            text.len()
        );
    }

    #[test]
    fn test_file_to_text_sorts_by_kind_then_name() {
        let file = make_file(vec![
            sym("Zebra", "struct", None),
            sym("alpha", "function", None),
            sym("Apple", "struct", None),
        ]);
        let text = file_to_text(&file);
        assert_eq!(
            text,
            "src/auth/middleware.rs in auth-service — function alpha, struct Apple, struct Zebra"
        );
    }

    #[test]
    fn test_symbol_to_text_without_signature() {
        let sym = SymbolForEmbedding {
            id: 2,
            name: "UserConfig".into(),
            kind: "interface".into(),
            signature: None,
            package: "shared-types".into(),
            file_path: "packages/shared-types/src/types.ts".into(),
        };
        let text = symbol_to_text(&sym);
        assert_eq!(
            text,
            "interface UserConfig in shared-types @ packages/shared-types/src/types.ts"
        );
    }
}
