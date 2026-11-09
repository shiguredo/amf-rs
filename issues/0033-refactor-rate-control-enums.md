# レート制御のマジックナンバーを enum 化する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
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
