# encode / decode のワーカー実装の重複を共通モジュールに統合する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-worker-common
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`src/encode.rs` と `src/decode.rs` のワーカー実装がほぼコピペになっている。`WorkerCommand`、ワーカーのループ、`drain_output`、`finish`、`Drop`、`SubmitInput` のリトライの 6 箇所が重複しており、片方だけ修正されて不整合になるリスクがある。

## 現状

- `WorkerCommand` が encode / decode で別々に定義されている。
- ワーカーのループ、`drain_output`、`finish`、`Drop`、`SubmitInput` のリトライが同じ構造で重複している。
- 重複している処理の一部は、型やエラー処理だけが異なる。
- 共通化の方針としてトレイトを作らない規約があるため、関数とクロージャで共通化する必要がある。

## 設計方針

- 共通のワーカーモジュールを作り、コマンド処理、出力回収、終了処理を関数として切り出す。
- 型やエラー処理の差分はクロージャまたは小さな構造体で渡す。トレイトは作らない。
- `SubmitInput` のリトライ方針 (回数、待機、`AMF_REPEAT` / `AMF_NEED_MORE_INPUT` の扱い) を共通化する。
- 出力と `user_data` の対応付けは、エンコーダーとデコーダーで異なる部分があるため、共通の枠組みに載せつつ個別のロジックは分離する。
- 既存の公開 API と挙動を変えない。リファクタリングの前後でテストが通ることを確認する。
- 重複が解消されたことを確認するため、encode / decode のワーカー部分の行数と構造を比較する。

## 完了条件

- ワーカーの共通部分が 1 箇所にまとまっている。
- encode / decode の差分が、型・出力変換・対応付けのロジックだけになっている。
- ワーカーが不要に `CodecConfig` 全体を複製していない。
- 既存のテストがすべて通る。
- `cargo clippy --workspace --all-targets -- -D warnings` が通る。
- `CHANGES.md` の `## develop` の `### misc` にリファクタリングとして追記する。
