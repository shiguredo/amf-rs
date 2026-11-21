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

- [CHANGE] `AmfLibrary::query_version` の戻り値を `AmfVersion` に変更する
  - `AmfVersion` の `major` / `minor` / `release` / `build` フィールドで各成分を取得できる
  - `Display` は `major.minor.release.build` 形式で表示する
  - @melpon
- [CHANGE] `Encoder::encode` の `EncodeOptions` を値で受け取るように変更する
  - @melpon
- [CHANGE] `FrameFormat::frame_size` を削除する
  - @melpon
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
- [CHANGE] `amf::Buffer::get_property` を削除する
  - プロパティの取得には `amf::Buffer::property_storage` から `amf::PropertyStorage::get_property` を使う
  - @melpon
- [CHANGE] `Encoder` が出力 `Buffer` の PTS を設定する挙動を削除する
  - 出力 `Buffer` の PTS は AMF が付けた値のままになる
  - 時刻が必要な場合は `user_data` などで利用者が管理する
  - @melpon
- [CHANGE] `EncoderConfig` に `b_pic_pattern` を追加し、B フレームを有効にできるようにする
  - `EncoderConfig` の公開フィールドを追加するため、構造体リテラルでの構築はコンパイルエラーになる
  - AMF の仕様上の値域は H.264 が 0-3、AV1 が 0-127 で、`0` は B フレームなし
  - B フレームに対応していない GPU で 1 以上を指定した場合と、B フレームを扱わない HEVC で指定した場合は `Encoder::new` がエラーを返す
  - `AMF_NEED_MORE_INPUT` を入力の受理として扱う
  - 入力フレームに一意な値を設定して入出力を対応付け、入力順と出力順が異なっても `user_data` を対応するフレームへ付ける
  - @melpon
- [CHANGE] `Error` を失敗の分類 enum に変更する
  - `Error` の `status()` / `new_custom` / `from_amf` / `check` を削除し、`InvalidArgument` / `InvalidState` / `Amf` / `Environment` / `Internal` のバリアントで失敗の種類を判別できるようにする
  - `AmfError` / `EnvironmentError` / `InternalError` / `InvalidArgumentError` / `InvalidStateError` を追加する
  - `Error::Amf(error)` から `error.status()` / `error.operation()` で AMF のステータスと操作名を取得する
  - 標準エラーから `Error` への `From` を実装し、`Error::source()` で内側の標準エラーを取得できるようにする
  - `Display` の形式を変更し、AMF のエラーは操作名とステータスを 1 回だけ表示する。Rust の関数名はメッセージに含めない
  - @melpon
- [CHANGE] `Decoder` が入力 `Buffer` の PTS を変更し、出力 `Surface` へ引き継ぐ挙動を削除する
  - 入力 `Buffer` の PTS は変更せず、出力 `Surface` の PTS も設定しない。出力 `Surface` の PTS は AMF が付けた値になる
  - 入出力の対応付けには `ShiguredoAmfFrameId` プロパティを使う。この名前はライブラリの予約名になる
  - @melpon
- [CHANGE] `Decoder::finish()` が残りの出力を回収できなかった場合に `Err` を返すように変更する
  - `Drain` の失敗時や、`AMF_EOF` の待機中の終端エラーで `Ok` を返していたのを `Err` にする
  - `Encoder::finish()` / `Decoder::finish()` がエラーを返した後の操作は `Drop` 以外を保証しない
  - @melpon
- [CHANGE] `Encoder` / `Decoder` を `finish` せずに drop した場合の挙動を変更する
  - drop の処理中に、すでに `encode()` / `decode()` が成功したフレームの出力が通知されることがある
  - drop は、すべてのフレームの出力が通知されることを保証しない
  - @melpon
- [ADD] `amf::Plane` / `amf::Context` / `amf::Component` / `amf::PropertyStorage` に `Debug` を実装する
  - 生ポインタはアドレスだけを表示する
  - @melpon
- [ADD] `PlaneType` を追加する
  - @melpon
- [ADD] `DataType` を追加する
  - `amf::Data::get_data_type()` の戻り値に使う
  - @melpon
- [ADD] `amf::Data` を追加する
  - `AMFData` の RAII ラッパーで、`Buffer` / `Surface` から `data()` / `into_data()` で `Data` を取得できる
  - `to_buffer` / `to_surface` で参照カウントを増やした `Buffer` / `Surface` を構築できる
  - `from_raw` / `from_raw_acquired` / `as_ptr` / `into_raw` / `property_storage` / `context` / `get_data_type() -> DataType` を提供する
  - @melpon
- [ADD] `amf::Buffer::property_storage` と `amf::PropertyStorage::get_property` / `get_property_int64` を追加する
  - @melpon
- [ADD] `amf::Buffer::as_slice` / `as_mut_slice` / `set_size` と `amf::Plane::as_slice` / `as_mut_slice` を追加する
  - `as_slice` / `as_mut_slice` は unsafe 関数とし、メモリの初期化と排他アクセスを呼び出し側の責任にする
  - null、サイズ、ピッチを検証し、スライスとして扱えない場合はエラーを返す
  - `Plane` のスライスは `hpitch * vpitch` バイトで、各行の先頭は `hpitch` バイト間隔になる
  - タイル状のプレーンはエラーを返す
  - `set_size` は確保済みサイズ以下の範囲でサイズを変更する
  - @melpon
- [ADD] `amf::Plane::is_tiled` と `amf::Buffer::get_memory_type` / `amf::Surface::get_memory_type` を追加する
  - @melpon
- [ADD] `amf::Buffer::set_pts` と `amf::Surface::get_pts` を追加する
  - @melpon
- [ADD] `amf::PropertyStorage::get_property_size` を追加する
  - @melpon
- [ADD] VP9 のデコードに対応する
  - `DecoderCodec::Vp9` を追加する
  - `VideoCodecType::Vp9` を追加し、`supported_codecs` に含める
  - @melpon
- [FIX] `Decoder::decode` の `user_data` が B フレームを含むストリームで誤ったフレームに付く問題を修正する
  - 入力 `Buffer` に一意な frame ID を設定し、出力 `Surface` の frame ID で `user_data` を引き当てる
  - 入力 `Buffer` の PTS を対応付けに使わず、変更もしない
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
- [FIX] `finish` 後の操作を拒否し、`Drop` の後始末を明確にする
  - `Encoder::encode()` / `Encoder::reconfigure()` / `Encoder::finish()` は `finish` 済みの場合に AMF を呼ばずにエラーを返す
  - `Encoder::finish()` / `Decoder::finish()` はワーカーの終了を join まで待つ
  - `Drop` ではハンドラーの panic と `Component::terminate` の失敗を `tracing::error!` で記録する
  - @melpon
- [FIX] `AmfLibrary::create_context` が `AMFContext1` の `InitVulkan` を取得できない場合に参照を解放しない問題を修正する
  - @melpon
- [FIX] 解像度が変化するストリームでもデコードを継続できるようにする
  - 解像度の変化を検出したら、保留中の出力を通知したあと新しい解像度でデコーダーを作り直し、変化を検出したフレームを再投入する
  - @melpon

### misc

- [UPDATE] レート制御メソッドの設定に bindgen の enum を使うようにする
  - H.264 / HEVC / AV1 のレート制御メソッドをコーデック別の enum へ変換して設定する
  - @melpon
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
- [UPDATE] `examples/amf-check` の `supported_codecs` 呼び出しを 1 回にまとめる
  - Vulkan デバイスの初期化とコーデックのプローブの重複をなくす
  - @melpon
- [UPDATE] Windows / macOS 向けの未使用の分岐を削除する
  - @melpon

- [UPDATE] encode / decode のワーカー実装を共通化する
  - コマンド処理、`QueryOutput` のポーリング、frame ID による入出力の対応付け、終了処理、`Drop` を `src/worker.rs` にまとめる
  - `QueryOutput` が返す出力は共通の `Data` 型で保持し、ハンドラーへの通知時に最終的なフレーム型へ変換する
  - 入力の frame ID の発行、入力プロパティへの設定、`SubmitInput` の呼び出しも共通化する
  - コマンドを 1 つ処理した後にも `QueryOutput` を回収する
  - `SubmitInput` のリトライ対象を `AMF_INPUT_FULL` と `AMF_DECODER_NO_FREE_SURFACES` に共通化し、エンコーダーの `AMF_REPEAT` はリトライせずエラーにする
  - `finish` はエンコーダー・デコーダーとも `AMF_EOF` まで出力を回収する
  - @melpon

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
