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

- [CHANGE] `supported_codecs` の戻り値と `CodecInfo` の能力情報を AMF が返す値に基づく形に変更する
  - `supported_codecs` は `Result<Vec<CodecInfo>, Error>` を返す
  - `CodecInfo` の `decoding` / `encoding` を `Option` に変更する
  - `DecodingInfo::supported` / `EncodingInfo::supported` を削除する
  - `hardware_accelerated` / `supports_multi_pass` / `profiles` を削除する
  - `EncodingProfiles` / `H264EncodingProfile` / `HevcEncodingProfile` / `Av1EncodingProfile` を削除する
  - `supports_frame_reordering` を `bool` から `Option<bool>` に変更する
  - `DecodingInfo` の `Eq` 実装を削除する
  - `max_profile: Option<VideoProfile>` / `max_level: Option<i64>` を追加する
  - `AccelerationType` / `SurfaceFormat` / `MemoryType` / `SurfaceFormatCapability` / `MemoryTypeCapability` / `IoCapabilities` / `Property` / `Value` を追加する
  - `properties` に AMF が返した全プロパティを追加する
  - @melpon
- [CHANGE] `amf::Plane` の寿命を取得元の `Surface` に縛るように変更する
  - `Plane<'a>` は `Surface` を借用し、`Surface` を drop した後の `Plane` の利用はコンパイルエラーになる
  - `Plane::from_raw` / `Plane::from_raw_acquired` は `Context` ではなく `&Surface` を受け取る
  - `Surface::convert` は `Plane` が生存している間はエラーを返すように変更する
  - `Surface::convert` の戻り値を `Result<(), Error>` に変更する
  - @melpon
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
- [CHANGE] HEVC で未対応の `qpb` / `gop_pic_size` を指定した場合はエラーを返すように変更する
  - `EncoderConfig` の `qpb` は `Encoder::new` がエラーを返す
  - `ReconfigureParams` の `qpb` / `gop_pic_size` は `Encoder::reconfigure` がエラーを返す
  - @melpon
- [CHANGE] `amf` モジュールのフレームデータ API から `ffi` の型を排除する
  - `Surface::get_plane` が `PlaneType` を、`Surface::convert` / `Context::alloc_surface` / `Context::alloc_buffer` が `MemoryType` を、`Context::alloc_surface` / `Component::init` が `SurfaceFormat` を受け取るように変更する
  - @melpon
- [ADD] `PlaneType` を追加する
  - @melpon
- [ADD] `amf::Buffer::as_slice` / `as_mut_slice` / `set_size` と `amf::Plane::as_slice` / `as_mut_slice` を追加する
  - `as_slice` / `as_mut_slice` は unsafe 関数とし、メモリの初期化と排他アクセスを呼び出し側の責任にする
  - null、サイズ、ピッチを検証し、スライスとして扱えない場合はエラーを返す
  - `Plane` のスライスは `hpitch * vpitch` バイトで、各行の先頭は `hpitch` バイト間隔になる
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
- [FIX] 10bit 系の入力フォーマットと HEVC Main10 でエンコーダーを初期化できるようにする
  - 入力フォーマットに応じて 8bit / 10bit のエンコードを設定する
  - HEVC の 10bit 入力と Main プロファイル (プロファイル未指定を含む) の組み合わせは `Encoder::new` がエラーを返す
  - AMF が対応しないフォーマットやプロファイルとの組み合わせは、AMF が返すエラーになる
  - @melpon
- [FIX] AV1 エンコーダーで `Av1Profile` を指定してもプロファイルが設定されない問題を修正する
  - @melpon
- [FIX] `QueryOutput` の終端エラーが通知されず、`finish()` が待ち続ける、またはタイムアウトする問題を修正する
  - `QueryOutput` の終端エラーをハンドラーへ通知し、ワーカーを終了する
  - エンコーダーの `finish()` で、対応する入力がない出力と、出力が得られなかった入力を通知する
  - エンコーダーの `finish()` の 5 秒タイムアウトを削除し、ワーカーの完了まで待つ
  - @melpon
- [FIX] `EncodeOptions::frame_type` で指定した `I` / `P` が無視される問題を修正する
  - `IDR` / `I` / `P` を `IDR`、`I`、`P` の順に優先して AMF の強制ピクチャタイプに対応付ける
  - `B` / `REF`、AV1 の `P`、未定義のビットは無視する
  - @melpon
- [FIX] AV1 の `qpb` が初期化時に反映されない問題を修正する
  - @melpon
- [FIX] AV1 の `PictureType` 判定で `INTRA_ONLY` を `P` と誤報告する問題を修正する
  - `KEY` / `INTRA_ONLY` / `INTER` を `Idr` / `I` / `P` に対応付ける
  - `SWITCH` / `SHOW_EXISTING` と未知の値は `Unknown` にする
  - @melpon

### misc

- [UPDATE] `ffi` の `AMF_SURFACE_FORMAT` / `AMF_MEMORY_TYPE` / `AMF_VARIANT_TYPE` を newtype enum に変更する
  - @melpon
- [UPDATE] shiguredo-rust と AGENTS.md の規約に合わせて lint 抑制・`.unwrap()`・ログ出力・コメントを整理する
  - `#[allow]` を `#[expect]` に置き換え、bindgen の生成コードを専用モジュールに閉じ込める
  - `log` を `tracing` に置き換え、毎フレーム実行されるログを `trace` に下げる
  - `.unwrap()` を `.expect("MESSAGE")` に置き換え、英語のコメントを日本語にする
  - @melpon
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
