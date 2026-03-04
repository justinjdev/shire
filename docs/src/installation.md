# Installation

## Homebrew (macOS, Linux)

```sh
brew tap justinjdev/shire
brew install shire
```

## From prebuilt binary

Download the latest release from [GitHub Releases](https://github.com/justinjdev/shire/releases) and add the binary to your PATH.

## From source

Requires [Rust](https://rustup.rs/) toolchain.

```sh
cargo install --path .

# With RAG vector search support (~30-50MB larger binary due to ONNX Runtime):
cargo install --path . --features rag
```
