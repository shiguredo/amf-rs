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

- [CHANGE] 最小サポート Rust バージョン (MSRV) を 1.88 から 1.95 に上げる
  - @voluntas
- [CHANGE] `amf::Context` 周りの整理
  - `init_vulkan` を非公開にし、`AmfLibrary::create_context` で Vulkan デバイスを初期化した `amf::Context` を作成できるようにする
  - `from_raw` / `from_raw_acquired` を非公開にする
  - `terminate` / `into_raw` を削除する
  - `PartialEq` / `Eq` を実装する
  - `Encoder::new` / `Decoder::new` が `Context` を受け取るように変更する
  - `amf` の各オブジェクトにコンテキストを取得する `context` 関数を追加する
  - @melpon
- [ADD] `amf::Buffer::set_pts` と `amf::Surface::get_pts` を追加する
  - @melpon
- [FIX] `Decoder::decode` の `user_data` が B フレームを含むストリームで誤ったフレームに付く問題を修正する
  - 入力 `Buffer` に一意な PTS を設定し、出力 `Surface` の PTS で `user_data` を引き当てる
  - 対応付けに使った PTS は、通知前に呼び出し元が設定した値へ戻す
  - `finish()` を呼び出した後に `decode()` を呼ぶとエラーを返す
  - `finish()` はワーカーの完了まで待つ
  - 複数フレームを含むバッファは `AMF_REPEAT` を検出してエラーを返す
  - @melpon
- [FIX] `amf` モジュールで `Clone` したハンドル経由のメソッド呼び出しがデータ競合する問題を修正し、コンポーネントに投入するオブジェクトが共有されていないことを確認する
  - @melpon
- [FIX] `AmfLibrary` / `amf::Context` / `Component::init` の AMF 呼び出しがデータ競合し、複数スレッドから Encoder / Decoder を並列に初期化した際に SIGSEGV する問題を修正する
  - @melpon

### misc

- [ADD] AMF ランタイムとコーデック対応状況を確認する `examples/amf-check` を追加する
  - @voluntas
- [UPDATE] CI / Release の GitHub Actions ランナーに Ubuntu 26.04 を追加する
  - @voluntas
- [UPDATE] `prek.toml` を shiguredo-rust 規約に合わせて整備する (builtin フック拡充、tombi、`cargo test` の pre-push 限定)
  - @voluntas
- [UPDATE] `rust-toolchain.toml` のツールチェインを MSRV の 1.95 に固定し、CI と devcontainer のツールチェインも 1.95 に統一する
  - @voluntas

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
