# テスト戦略と構成を規約に合わせ、欠落しているテストを追加する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-test-suite
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

テストの構成が shiguredo-rust の規約と一致しておらず、主要な公開 API のテストが欠けている。テストの配置・命名・分割を規約に合わせ、回帰を検出できるテストを追加する。

## 現状

- `tests/test_roundtrip.rs` は約 1600 行あり、`src/roundtrip.rs` に対応しないモジュール非対応のテストに `test_` プレフィックスが付いている。ファイル内の `mod` 分割もない。
- `tests/test_decode.rs` が存在せず、`FnEncodeHandler` / `FnDecodeHandler` の単体テストもない。過去の設計で求められたハンドラー単体テストが未実施のままである。
- `tests/test_codec_info.rs` の `test_supported_codecs_hw_accel_consistency` は、実装が `supported` と同じ値を `hardware_accelerated` に入れているため、常に成功する。
- `tests/test_roundtrip.rs` のコピーヘルパーは `get_native()` の null チェックと `hpitch >= 行バイト数` の確認をしない。`buffer_to_slice` は null とサイズを検証しない。CODEBASE.md が求める安全境界チェックに反する。
- YV12 / I420 のテストデータは U と V をともに 128 で埋めるため、U/V の入れ替わりを検出できない。PSNR は Y プレーンだけを比較している。
- `Encoder::new` の入力検証 (width が 0、奇数、framerate が 0 など) は GPU 不要でテストできるが、テストがない。
- 10bit 系フォーマットのテストは別 issue、Decoder の `user_data` 対応テストも別 issue の範囲とする。

## 設計方針

- `tests/test_roundtrip.rs` をモジュールに対応しない名前に変更し、内部を `mod` で分割する。共有ヘルパーは `tests/helpers/` に置く。
- `tests/test_decode.rs` を追加し、`FnDecodeHandler` の `on_decoded` 呼び出しと `Error` 型の差し替えを検証する。`tests/test_encode.rs` にも `FnEncodeHandler` のテストを追加する。
- `test_supported_codecs_hw_accel_consistency` を、実装の内部整合性ではなく実際の AMF 応答に対する検証に置き換える。検証できない場合は削除する。
- コピーヘルパーに null、サイズ、`hpitch`、`vpitch` の検証を追加し、日本語のメッセージで明示的に失敗させる。
- YV12 と I420 のテストデータで U と V に異なる値を設定し、デコード後のクロマプレーンを比較して U/V の順序を検証する。
- `Encoder::new` の入力検証のテストを追加する。
- テストファイルが長くなりすぎないように分割し、`#[ignore]` は使わない。

## 完了条件

- テストのファイル名・配置・分割が規約に一致する。
- `tests/test_decode.rs` とハンドラー単体テストが追加される。
- 恒真のテストが実値検証または削除になる。
- `tests/test_encode.rs` の自明なテストを削除または置換し、`tests/test_roundtrip.rs` の重複ヘルパー、`_ => unimplemented!()`、空の doc を整理する。
- コピーヘルパーが安全境界チェックを行う。
- YV12 / I420 のクロマ検証と `Encoder::new` の検証テストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` の `### misc` にテスト改善として追記する。
