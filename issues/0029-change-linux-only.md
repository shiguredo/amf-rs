# Linux x86_64 以外を `compile_error!` で拒否し、Windows / macOS の死にコードを削除する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/change-linux-only
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`src/dl.rs` には Windows と macOS の分岐があるが、どちらも動作しない。`AMF_DLL_NAME` は Linux の `libamfrt64.so.1` 固定で、`to_wstring` は UTF-32 を前提としている。Windows の `LoadLibraryW` や macOS の `RTLD_LOCAL` 分岐は到達しても失敗する死にコードであり、保守の負担と誤解の原因になる。

## 現状

- `src/dl.rs` に `#[cfg(windows)] mod windows` と macOS の `RTLD_LOCAL` 分岐がある。
- `src/sys.rs` の `AMF_DLL_NAME` は `libamfrt64.so.1` で、Windows / macOS のライブラリ名と一致しない。
- `to_wstring` は UTF-32 を前提としており、Windows の UTF-16 と一致しない。
- サポート対象外のプラットフォームでビルドすると、実行時にしか失敗しない。
- README / Cargo.toml の対応プラットフォームの記述が実装と一致しているか確認されていない。

## 設計方針

- `#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]` で `compile_error!` を出し、サポート対象を Linux x86_64 に限定する。
- `src/dl.rs` の Windows / macOS 分岐を削除し、Linux の `dlopen` 経路だけにする。
- `AMF_DLL_NAME` と `to_wstring` の前提をコメントで明記する。
- README / Cargo.toml の `categories` / `keywords` など、プラットフォームに関する記述を実装に合わせる。
- Linux x86_64 で `cargo build` / `cargo test` が通ることを確認する。
- サポート対象外のターゲットで `cargo check` したときに `compile_error!` が出ることを確認する。

## 完了条件

- Linux x86_64 以外のターゲットでビルドすると、明確な `compile_error!` が出る。
- Windows / macOS の死にコードが削除されている。
- README の対応プラットフォームの記述が実装と一致する。
- Linux x86_64 で `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[CHANGE]` を追記する。
