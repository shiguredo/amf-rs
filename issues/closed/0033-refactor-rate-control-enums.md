# レート制御のマジックナンバーを enum 化する

- Created: 2026-11-10
- Completed: 2026-11-19
- Branch: feature/refactor-rate-control-enums
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`src/encode.rs` のレート制御プロパティの設定に、数値のマジックナンバーを使った表が 3 つある。AMF の enum 値と対応しているが、数値のままでは意味が分からず、AMF SDK の更新で値が変わった場合に追随できない。

## 現状

- H.264 / HEVC / AV1 のレート制御プロパティ (rate control method、quality preset など) を数値の表で設定している。
- bindgen で生成した enum を rustify していないため、数値と enum の対応がコード上で分からない。
- 表が 3 つに分かれており、同じような数値が重複している。
- 数値の根拠がコメントされていない。

## 設計方針

- 使用する `AMF_VIDEO_ENCODER_*_RATE_CONTROL_METHOD` などの enum を bindgen の rustify 対象に追加する。
- 数値の表を `match` に置き換え、enum の variant 名で設定する。
- コーデックごとに異なる enum は、共通の内部表現に変換してから設定する。
- 数値のまま残す必要がある場合は、const に名前を付け、根拠をコメントする。
- 既存の設定値が変わらないことをテストで確認する。

## 完了条件

- レート制御の設定に数値のマジックナンバーが残っていない。
- bindgen の enum を使って設定している。
- 既存のエンコード結果が変わらない。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` の `### misc` にリファクタリングとして追記する。

## 解決方法

- `build.rs` の bindgen 設定に `AMF_VIDEO_ENCODER_RATE_CONTROL_METHOD_ENUM`、`AMF_VIDEO_ENCODER_HEVC_RATE_CONTROL_METHOD_ENUM`、`AMF_VIDEO_ENCODER_AV1_RATE_CONTROL_METHOD_ENUM` の `rustified_enum` を追加した。
- `RateControlMode` にコーデック別の変換メソッド `to_h264` / `to_hevc` / `to_av1` を追加し、AMF の enum variant へ変換してから `AMF_VIDEO_ENCODER_*_RATE_CONTROL_METHOD` に設定するようにした。レート制御メソッドの数値リテラルは削除した。
- 生成された enum の判別子が旧数値表と一致することを確認した。H.264 は 0-6、HEVC と AV1 は 0, 3, 2, 1, 4, 5, 6 である。
- docs.rs ビルド用のダミーバインディングに 3 つの enum を追加した。
- H.264 / HEVC / AV1 の全 7 レート制御モードでエンコーダーを作成し、各 3 フレームをエンコードできることを確認する実機テストを `tests/test_encode.rs` に追加した。
- AMD GPU 実機で `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` を実行し、全 115 テストが成功した。`DOCS_RS=1 cargo doc --no-deps` も成功した。
- `CHANGES.md` の `## develop` の `### misc` に `[UPDATE]` を追記した。
