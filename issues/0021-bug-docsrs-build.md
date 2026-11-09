# `build.rs` の DOCS_RS ダミーと環境変数の取り扱いを修正する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-docsrs-build
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`build.rs` の DOCS_RS 向けダミーバインディングが現行のコードに対して不足しており、`DOCS_RS=1 cargo check --lib` が 163 エラーで失敗する。`cargo doc` は rustdoc が関数本体や定数を型検査しないため通るが、CI の docs-rs ジョブはこの失敗を検出できない。

また、`cargo::rerun-if-env-changed=DOCS_RS` がないため、同じ target ディレクトリで `DOCS_RS` を切り替えても build script が再実行されない。README の手順どおり `DOCS_RS=1 cargo doc` を実行した後に `cargo check` すると、スタブの `properties.rs` を流用して `sys::str` の未定義エラーになる。

さらに、`#define  NAME L"..."` のように `#define` の後に空白が 2 個ある定義を取りこぼす。実際に `AMF_VIDEO_ENCODER_INPUT_QUEUE_SIZE` が生成されていない。

## 現状

- `build.rs` の `write_docs_rs_bindings` が生成するダミーは、`AMF_RESULT` や `AMF_SURFACE_FORMAT` の一部の値、vtable 構造体、`sys::str` のプロパティ定数を持たない。
- `build.rs` は `rerun-if-changed` を `Cargo.toml` と `build.rs` にだけ設定しており、`DOCS_RS` の変化を追跡していない。
- `generate_wide_string_constants` は `strip_prefix("#define ")` の直後に 1 個の空白だけを想定しており、空白が 2 個以上の定義を読み飛ばす。
- `git clone` は `--branch v1.5.2` のタグ指定のみで、commit 固定や取得物のハッシュ検証がない。
- CI の docs-rs ジョブは `cargo doc --no-deps` だけで、`cargo check` を実行していない。

## 設計方針

- DOCS_RS 向けのダミーを、少なくとも `DOCS_RS=1 cargo check --lib` が通る完全さにする。難しければ、DOCS_RS 時は `sys` の FFI 実装を `cfg` でダミーに差し替え、手書きコードが実バインディングの型に依存しないようにする。
- `println!("cargo::rerun-if-env-changed=DOCS_RS")` を追加する。
- `#define` のパーサを、空白が 1 個以上ある場合に対応させる。`#ifdef` を評価しないことと、同名マクロが最初の定義だけ採用されることは、対象プラットフォームの定義だけを抽出するようにするか、制約としてコメントに明記する。
- CI に `DOCS_RS=1 cargo check --lib` を追加し、docs-rs 経路の型検査を行う。
- `git clone` は commit SHA 固定と取得物の検証を行う。難しければ、少なくともタグが可変であるリスクを doc に書く。
- パーサの単体テストを追加する (空白 2 個、行継続、同名定義)。

## 完了条件

- `DOCS_RS=1 cargo check --lib` が成功する。
- `DOCS_RS` の有無を切り替えても build script が再実行され、古いバインディングを流用しない。
- `AMF_VIDEO_ENCODER_INPUT_QUEUE_SIZE` を含む、空白が 2 個以上の定義が生成される。
- CI に `DOCS_RS=1 cargo check --lib` が追加される。
- パーサの単体テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` の `### misc` にビルド修正として追記する。
