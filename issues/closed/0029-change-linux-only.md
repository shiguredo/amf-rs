# Windows / macOS の死にコードを削除し、対応プラットフォームを README に明記する

- Created: 2026-11-10
- Completed: 2026-11-19
- Branch: feature/change-linux-only
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`src/dl.rs` には Windows と macOS 向けの分岐があるが、どちらもこのクレートでは動作しない。

Windows には AMF ランタイムが存在するため、分岐を残せば将来の Windows 対応に使えるように見える。しかし現状の Windows 分岐はローダーだけで、`AMF_DLL_NAME` は Linux の `libamfrt64.so.1` 固定、`to_wstring` は UTF-32 前提であり、Windows の `wchar_t` (UTF-16) と一致しない。
Windows 上でビルドすると bindgen が生成する `wchar_t` も `u16` になるため、`to_wstring` の戻り値をそのまま渡すこともできない。macOS には AMF ランタイムが配布されていない。

動作しない分岐を残すと、対応済みに見えたり、変更のたびに考慮が必要になったりする。分岐を削除し、対応プラットフォームを README で明示する。

## 現状

- `src/dl.rs` に `#[cfg(windows)] mod windows` と、macOS 用の `RTLD_LOCAL` 分岐がある。
- `src/sys.rs` の `AMF_DLL_NAME` は `libamfrt64.so.1` で、Windows の `amfrt64.dll` と一致しない。
- `src/sys.rs` の `to_wstring` / `from_wstring` は UTF-32 を前提としており、Windows の UTF-16 と一致しない。
- 非対応プラットフォーム向けのビルドは、実行時まで失敗に気付かないか、原因の分かりにくい型エラーになる。
- README は動作要件として Linux (x86_64) を挙げているが、Windows / macOS が未対応であることは明記していない。

## 設計方針

- `src/dl.rs` から Windows と macOS 向けの分岐を削除し、Linux で使う `dlopen` 経路だけにする。
- `compile_error!` などで非対応プラットフォームを明示的に拒否しない。検出と案内の仕組みを維持するコストが、得られる利益を上回るため。対応プラットフォームは README で明示する。
- `AMF_DLL_NAME` と `to_wstring` / `from_wstring` が Linux x86_64 前提であることをコメントで明記する。
- README に Windows / macOS が未対応であることを明記する。
- 将来 Windows に対応する場合は、削除した分岐を復元するのではなく、ライブラリ名、ワイド文字列、CI と実機での検証を含めて改めて設計する。
- `Cargo.toml` にプラットフォームに関するメタデータは無いため変更しない。
- `CHANGES.md` の `## develop` の `### misc` に、Windows / macOS 向けの未使用の分岐を削除する旨を `[UPDATE]` で追記する。
- Linux x86_64 で `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通ることを確認する。

## 完了条件

- `src/dl.rs` に Windows 向け、macOS 向けの分岐が残っていない。
- `AMF_DLL_NAME` と `to_wstring` / `from_wstring` のコメントから、Linux x86_64 前提であることが読み取れる。
- README に Windows / macOS が未対応であることが書かれている。
- `CHANGES.md` の `## develop` の `### misc` に `[UPDATE]` が追記されている。
- Linux x86_64 で `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通る。

## 解決方法

`src/dl.rs` から Windows 向けの `#[cfg(windows)] mod windows` と macOS 向けの `RTLD_LOCAL` 分岐を削除し、`DynLib` が Linux の `dlopen` だけを使う構成にした。`AMF_DLL_NAME` と `to_wstring` / `from_wstring` のコメントには Linux x86_64 前提を明記した。

README の動作要件に Windows と macOS が未対応であることを追記し、`CHANGES.md` の `## develop` の `### misc` に `[UPDATE]` を追加した。

Linux x86_64 の AMD GPU 環境で `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通ることを確認した (114 テスト成功)。
