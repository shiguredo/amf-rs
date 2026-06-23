# 変更履歴

- UPDATE
  - 後方互換がある変更
- ADD
  - 後方互換がある追加
- CHANGE
  - 後方互換のない変更
- FIX
  - バグ修正

## develop

## 2026.3.0

**リリース日**: 2026-06-23

- [CHANGE] Encoder の出力取得を next_frame() ポーリングから非同期のトレイトベースのハンドラー方式に変更する
  - @melpon
- [CHANGE] Decoder の出力取得を next_frame() ポーリングから非同期のトレイトベースのハンドラー方式に変更する
  - @melpon
- [CHANGE] encode/decode の入出力を Surface/Buffer に変更し、EncodedFrame/DecodedFrame に T 型パラメータを追加する
  - @melpon
- [ADD] amf::* に AMF オブジェクトの Rust ラッパーを追加する
  - @melpon
- [UPDATE] AMF を v1.5.0 から v1.5.2 に更新する
  - @voluntas
- [FIX] 別々のインスタンスから異なるスレッドで操作したときに SIGSEGV が発生する問題を修正する
  - @melpon

### misc

- [ADD] macOS の Apple Container 上で Rosetta 2 を使わず x86_64 ターゲットの clippy を実行できるよう `.devcontainer/Dockerfile` を更新し、`make container-build` / `prek` の OS 分岐 `cargo-clippy` フックを追加する
  - @voluntas

## 2026.2.0

**リリース日**: 2026-04-15

- [ADD] Encoder に再初期化なしで動的プロパティを再設定する `reconfigure` API を追加する
  - @melpon
- [FIX] キーフレーム生成時に SPS/PPS ヘッダーが付与されていなかったのを修正
  - @melpon

## 2026.1.0

**リリース日**: 2026-03-31
