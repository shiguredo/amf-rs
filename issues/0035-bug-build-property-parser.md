# `build.rs` のプロパティ抽出を正確にする

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-build-property-parser
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`build.rs` の `#define` 抽出が、`#ifdef` の評価、行継続、空白の扱いを十分に処理していない。対象プラットフォームと異なる定義を採用したり、定義を取りこぼしたりする。`BUILD_METADATA_REPOSITORY` や `AMF_DX_VERSION` のような未使用の項目も残っている。

## 現状

- `#ifdef` を評価しないため、`AMF_VIDEO_DECODER_NATIVEWINDOW` は Android 用の値を Linux ビルドでも採用する。
- 行継続 (`\`) に対応していない。
- `#define` の後の空白が 1 個であることを前提にしている箇所がある。
- `BUILD_METADATA_REPOSITORY` が未使用である。
- `AMF_DX_VERSION` は Windows 専用で未使用だが rustify している。
- DOCS_RS 向けのダミー生成は別 issue で扱う。

## 設計方針

- `#ifdef` / `#ifndef` / `#if defined(...)` のうち、対象プラットフォームに関係するものだけを評価する。完全な C プリプロセッサを実装するのではなく、AMF のヘッダーで使われている構文に限定する。
- 行継続 (`\`) を結合してから `#define` を解析する。
- `#define` 後の空白を 1 個以上として扱う。
- `BUILD_METADATA_REPOSITORY` を削除するか、実際に使用する。
- `AMF_DX_VERSION` は Windows 専用であることを明記し、Linux ビルドでは rustify しない。
- パーサの単体テストを追加する。空白 2 個、行継続、`#ifdef` の分岐、同名定義の最初勝ちを検証する。

## 完了条件

- Linux ビルドで Android / Windows 専用の定義を採用しない。
- 行継続と空白 2 個以上の `#define` を正しく抽出する。
- 未使用の `BUILD_METADATA_REPOSITORY` と `AMF_DX_VERSION` の扱いが決まっている。
- パーサの単体テストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` の `### misc` にビルド修正として追記する。
