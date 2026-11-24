# `examples/amf-check` の `supported_codecs` 二重呼び出しを修正する

- Created: 2026-11-10
- Completed: 2026-11-18
- Branch: feature/fix-amf-check-probe
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`examples/amf-check` が `supported_codecs()` を 2 回呼んでおり、呼び出しごとに Vulkan デバイスの初期化を含むコーデックのプローブが走る。実行時間とログが無駄に増え、GPU リソースも余分に消費する。

## 現状

- `examples/amf-check/src/main.rs` の 2 箇所で `supported_codecs()` を呼んでいる。
- `supported_codecs()` は呼び出しごとに `Context` と Vulkan デバイスを作る。
- 2 回の結果が同じであることを前提にした表示になっている。
- 呼び出し回数を検証するテストはない。

## 設計方針

- `supported_codecs()` を 1 回だけ呼び、結果を変数に保持して両方の表示で使う。
- 1 回の呼び出しで必要な情報が揃っていることを確認する。足りない場合は `supported_codecs()` の戻り値を拡張する。
- 出力が変わらないことを確認する。
- 可能なら、`supported_codecs()` の呼び出し回数を数えるテスト、または `amf-check` の出力を検証するテストを追加する。

## 完了条件

- `supported_codecs()` の呼び出しが 1 回になっている。
- `amf-check` の出力が従来と同等である。
- 二重初期化がなくなったことを確認できるテスト、または実行ログの確認結果が issue に記録される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` の `### misc` に example の修正として追記する。

## 解決方法

`main` で `supported_codecs()` を 1 回だけ呼び、その結果を `print_codec_support` と `check_component_init` の両方に渡すようにした。結果は `supported_codecs()` のエラーを CLI の `Error` に包んだ `Result<Vec<CodecInfo>>` として共有するため、取得に失敗した場合も従来と同じ 2 箇所のメッセージを出力する。

実機 (AMD Radeon 780M / AMF 1.5.3) の `~/melpon/amf-rs` で修正前後を 3 回ずつ実行して比較した。標準出力は完全に一致し、AMF デコーダーが出力する Vulkan 関連のエラー行は 20 行から 10 行に半減した。実行時間は 0.21 秒から 0.18 秒になった。

- 修正前: stderr は AMF のエラー行 20 行と `elapsed=0.21` (3 回とも)
- 修正後: stderr は AMF のエラー行 10 行と `elapsed=0.18` (3 回とも)
- `diff` による標準出力の比較で差分なし

呼び出し回数を数えるテストは、`supported_codecs()` を差し替えるモックが必要になるため追加していない。実機の実行ログでプローブが 1 回になったことを確認した。`cargo test --workspace` は 110 件すべて成功し、`cargo fmt --all -- --check` と `cargo clippy --workspace --all-targets -- -D warnings` も成功した。
