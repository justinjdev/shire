# Installation

## Homebrew (macOS, Linux)

```sh
brew tap justinjdev/shire
brew install shire
```

## From prebuilt binary

Download the latest release from [GitHub Releases](https://github.com/justinjdev/shire/releases) and add the binary to your PATH.

## Nix

```sh
# Install into your profile
nix profile install github:justinjdev/shire

# Or run without installing
nix run github:justinjdev/shire
```

## From source

Requires [Rust](https://rustup.rs/) toolchain.

```sh
cargo install --path .
```
