# デコード出力フォーマットの選択と実効フォーマットの取得に対応する

- Created: 2026-11-10
- Refreshed: 2026-11-20
- Reporter: @melpon

## 目的

`Decoder` は常に NV12 で初期化するため、HEVC Main10 や AV1 10bit のデコード結果を 10bit のまま取得できない。また `DecodedFrame` に実効ピクセルフォーマットの accessor がない。幅・高さは `surface()` の `Plane` から取得できるが、実効ピクセルフォーマットを取得する安全な API はない。vpl-rs は `DecodedFrame::frame_format()` / `width()` / `height()` を返し、nvcodec-rs は `DecoderConfig::surface_format` を、video-toolbox-rs は `DecoderConfig::pixel_format` を持つ。amf-rs にも出力フォーマットを指定する機能と、実効フォーマットを取得する API が必要である。

## 現状

- `src/decode.rs` の `Decoder::new` は `component.init(SurfaceFormat::from(FrameFormat::Nv12), 0, 0)` を固定で呼ぶ。解像度変化時の `reinitialize_after_resolution_change` も `.init(SurfaceFormat::from(FrameFormat::Nv12), width, height)` で NV12 固定である。
- `DecoderConfig` は `codec` だけを持ち、出力フォーマットを指定できない。利用側は `DecoderConfig { codec }` の構造体リテラルで構築している。
- `DecodedFrame` は `surface()` / `user_data()` / `into_parts()` を公開するが、幅・高さ・ピクセルフォーマットを返さない。`Surface` にも `get_format()` はない。
- `Plane::get_width` / `get_height` は AMF の仕様では crop region の幅・高さを返し、crop が未設定の場合はサーフェス全体の寸法を返す。実機 (AMD Radeon 780M / AMF ランタイム 1.5.3.0) の H.264 1080p では、NV12 の Y プレーンが 1920x1080、`get_hpitch` が 2048、`get_vpitch` が 1088 だった。P010 / P012 / P016 の `get_vpitch` は 1080、I420 の `get_vpitch` も 1080 であり、フォーマットによってプレーンのピッチが異なる。
- `AMF_VIDEO_DECODER_ALLOC_SIZE` は AMF の仕様では割り当て済み出力サーフェスのサイズ (既定 `(1920, 1088)`) を返すが、コードでは使っていない。実機では H.264 1080p の NV12 では (1920, 1088) で、プレーンの `get_vpitch` も 1088 だった。P010 では `AMF_VIDEO_DECODER_ALLOC_SIZE` の高さ 1088 に対してプレーンの `get_vpitch` が 1080 で一致しなかった。HEVC Main10 720p でも (1920, 1088) を返し、プレーンの `get_width` / `get_height` (1280x720) と一致しなかった。
- `reinitialize_after_resolution_change` は `AMF_VIDEO_DECODER_CURRENT_SIZE` を `PropertyStorage::get_property_size` で読み、解像度変化後の再初期化に使っている。実機では H.264 1080p で (1920, 1080)、HEVC Main10 720p で (1280, 720) を返した。
- `AMF_VIDEO_DECODER_OUTPUT_FORMAT` は `sys::str` に定数があるが、コードでは使っていない。実機では Init に NV12 を渡すと 1 (NV12)、P010 を渡すと 10 (P010) を返し、出力サーフェスのフォーマットと一致した。HEVC Main10 のストリームに NV12 を渡した場合は AMF が NV12 に変換し、`AMF_VIDEO_DECODER_OUTPUT_FORMAT` も NV12 を返した。
- H.264 / HEVC / AV1 のデコーダーは `Init` が NV12 / P010 / P012 / P016 / YUV420P (`FrameFormat::I420`) で `AMF_OK` を返した (実機、AMF ランタイム 1.5.3.0)。H.264 1080p では 5 形式すべてで要求した形式の出力サーフェスが返り、`AMF_VIDEO_DECODER_OUTPUT_FORMAT` もそれぞれ 1 / 10 / 19 / 20 / 7 だった。HEVC Main10 と AV1 10bit では、現在のコンポーネント ID (`AMFVideoDecoderHW_H265_HEVC` / `AMFVideoDecoderHW_AV1`) で P010 出力を確認した。AMF SDK v1.5.2 の Programming Guide は「全コーデックで NV12 とし、Motion JPEG だけ YUY2 も利用できる」としているが、実機の結果と一致しない。
- 現在のワーカーは出力 `Surface` を `convert(MemoryType::Host)` してからハンドラーへ渡す。`DecodedFrame::surface()` は Host メモリの `Surface` を返す。
- `issues/closed/0023-change-safe-frame-api.md` で `Plane::as_slice` / `Buffer::as_slice` / `SurfaceFormat` / `PlaneType` が追加され、公開 API から `ffi` 型が排除された。0023 は完了済みで、出力フォーマットの選択と `DecodedFrame` の accessor は扱っていない。
- `issues/closed/0011-bug-10bit-input-formats.md` で 10bit 入力と HEVC Main10 のエンコードに対応した。現在の `Encoder` で HEVC Main10 / AV1 の P010 入力から 10bit テストストリームを生成できる。
- `supported_codecs()` の `DecodingInfo::output.formats` で AMF が報告する出力フォーマットを照会できる。実機では HEVC の一覧に P010 が含まれない一方、Init(P010) と P010 出力は成功した。この環境の HEVC では capability の値が実際に使える形式と一致しないため、capability の値だけでは利用可否を判定できない。
- README と `skills/shiguredo-amf/SKILL.md` はデコード出力を NV12 と記載している。

## 設計方針

- `DecoderConfig` に `output_format` を追加する。既定は NV12 (`FrameFormat::Nv12` 相当) とし、既存の利用者の実行時挙動を変えない。`DecoderConfig::new(codec)` を追加して `output_format` を NV12 で初期化し、利用者は公開フィールドを書き換えて指定する。公開フィールドの追加は構造体リテラルを壊すため、README / SKILL.md / tests / examples の構築箇所を移行する。`CHANGES.md` には `[CHANGE]` として記録する。
- 出力形式の型は `SurfaceFormat` (`Known(FrameFormat)` / `Unknown(u32)`) を採用する。`FrameFormat` の `I420` は `AMF_SURFACE_YUV420P` に対応する。`FrameFormat` の doc は「エンコーダーの入力フレームフォーマット」から、エンコーダー入力とデコーダー出力の共用に更新する。`Init` の結果で利用可否を判定する。
- `Decoder::new` と `reinitialize_after_resolution_change` の両方で、設定した `output_format` を `component.init` に渡す。`Decoder` は再初期化のために `output_format` を保持する。
- `DecodedFrame` に `format()` を追加する。フォーマットは出力 Surface の `get_format()` から取得し、`DecodedFrame` の構築時に確定する。幅・高さは `Surface` の Y プレーンから取得できるため、`DecodedFrame` には追加しない。
- フォーマットは要求値ではなく実効値を使う。`AMFSurface::GetFormat` を `Surface::get_format()` として公開し、deliver で Surface から読む方法を第一候補にする。`AMF_VIDEO_DECODER_OUTPUT_FORMAT` を deliver で読む場合は `Component` の Clone を deliver クロージャに捕捉する必要があるため、Init 後や解像度変化後の確認に使う。実機では `Surface::get_format()` と `AMF_VIDEO_DECODER_OUTPUT_FORMAT` が一致した。未知の形式を返せるよう、戻り値は `SurfaceFormat` にする。
- `Init` が AMF のエラーを返した場合は `Error::Amf(AmfError)` とし、`AmfError::status()` / `operation()` から状態と操作を取得できるようにする。要求したフォーマットをエラーから判別する専用 API は設けない。呼び出し元が `DecoderConfig` を保持しているため、追加の API は不要と判断した。
- `output_format` の事前検証は追加せず、`Init` の結果で判定する。0027 で設定値の事前検証をしない方針が決まっていることと揃える。`supported_codecs()` の capability も事前検証には使わない。
- 出力 Surface のプレーンは、既存の `Plane::get_width` / `get_height` / `get_hpitch` / `get_vpitch` と 0023 で追加された `Plane::as_slice` で読み取れる。`Surface::convert(MemoryType::Host)` はハンドラー通知前に実行済みで、`Plane` が生存している間は `convert` がエラーになる。
- `Plane` は crop の offset (`GetOffsetX` / `GetOffsetY`) を公開していない。`format()` には offset は不要なため、本 issue のスコープ外とする。crop offset が必要な場合は別 issue にする。
- 実機テストを追加する。H.264 1080p で Y プレーンの `get_width()` / `get_height()` が 1920x1080 になり、NV12 の `get_vpitch` が 1088 であること (crop 後の高さ 1080 とプレーンのピッチを区別する)、P010 / P012 / P016 / I420 でも要求した形式の Surface が返ること、HEVC Main10 と AV1 10bit で P010 出力が得られること、HEVC Main10 に NV12 を要求した場合に AMF が変換した NV12 が返ることを検証する。解像度変化をまたいでも `format()` が設定値のままであることと、変化前後の各フレームで Y プレーンの幅・高さが正しいことも検証する。10bit と複数解像度のテストストリームは現在の `Encoder` で生成できる。
- `src/decode.rs` の `DecoderConfig::codec` の doc から「出力フォーマットは NV12 に固定される」を削除し、`output_format` の説明に置き換える。`Decoder::new` の `# Errors` には、`CODEBASE.md` のエラー契約に従って、バックエンドが指定した出力形式に対応していない場合を含める。
- README と `skills/shiguredo-amf/SKILL.md` の「デコード出力は NV12」を更新する。README の特徴・デコード出力フォーマット表、SKILL.md の特徴・API 表に加え、両方のデコード例を新しい API (`DecoderConfig::output_format` と `DecodedFrame::format()`) に置き換え、表示サイズは Y プレーンから取得する例にする。

## 完了条件

- `DecoderConfig::output_format` でデコード出力フォーマットを選択できる。
- 解像度変化後も設定した出力フォーマットが維持される。
- `DecodedFrame::format()` が実効フォーマットを返す。
- 解像度変化をまたいでも、各フレームの Y プレーンの幅・高さが正しい。
- 既定の NV12 で既存の実行時挙動が変わらない。
- README / SKILL.md / tests / examples の `DecoderConfig` 構築箇所が新しい API に追随する。
- P010 / P012 / P016 / I420 で要求した形式の Surface が返ることを実機テストで確認する。
- HEVC Main10 と AV1 10bit の P010 出力を検証する実機テストが追加される。
- README と `SKILL.md` のデコード出力の記述とコード例が実装と一致する。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[CHANGE]` と `[ADD]` を追記する。

## 実装状況

- 実装はブランチ `feature/add-decoder-output-format` のコミット `066355e` に保全している。マージはしていない。
- `DecoderConfig::output_format`、`DecoderConfig::new`、`Surface::get_format()`、`DecodedFrame::format()` を追加し、解像度変化後の再初期化でも `output_format` を使うようにした。
- 表示サイズは `DecodedFrame` の `width()` / `height()` ではなく、`surface()` の Y プレーンの `get_width()` / `get_height()` から取得する方針に変更した。`AMF_VIDEO_DECODER_CURRENT_SIZE` のフォールバックは不要と判断して削除した。
- README、`skills/shiguredo-amf/SKILL.md`、`CHANGES.md`、examples、テストを更新した。
- ローカルの `cargo fmt` / `cargo clippy` / `cargo build` / `cargo test --workspace --no-run` は成功している。
- レビューで未対応の項目がある。`FrameFormat` の型 doc と SKILL の型表が「エンコーダー入力」のまま、README / SKILL のデコード例に `output_format` の指定例がない、テストの GPU 能力依存、develop とのマージ衝突である。実装を再開する際に対応する。

## 調査内容

- 実機 (AMD Radeon 780M / AMF ランタイム 1.5.3.0 / amf-amdgpu-pro 26.20.618-1) で、非 NV12 出力のデコードを繰り返すと GPUVM page fault と device reset が発生することを確認した。
- 1080p H.264 の 10 フレームを I420 出力で一気にデコードし、フレームを保持せず破棄し、`convert(MemoryType::Host)` も無効化した条件で、20 回中 6 回 page fault が発生した。NV12 出力の同じ条件では 20 回中 0 回だった。
- 出力を 1 フレームずつ待ってから次を投入すると、I420 出力の同じ条件で 20 回中 0 回だった。
- P010 出力では 20 回中 1 回、I420 出力では 320x240 の 60 フレームでも 20 回中 2 回 page fault が発生した。解像度には依存しない。
- エンコーダーを排除した静的フィクスチャ (320x320、50 フレーム) の I420 デコードでも 20 回中 3 回発生した。フレームの保持、Host 変換、エンコーダーは原因ではない。
- 1080p の 5 形式を 10 フレームずつデコードするテストでは、I420 の変換開始直後に 10 回中 2 回 SIGSEGV した。P010 / P012 / P016 は完了した。
- kernel log では `amf-decoder-wor` スレッドで page fault、`ring timeout`、`device wedged` が記録された。AMF のログには `Blt420Compute::SplitUV`、`AMFVideoConverterImpl::Process`、`m_pConverter->QueryOutput`、`vkQueueSubmit err=-4` が出ることがあった。
- テストプロセスは page fault が発生しても exit 0 で成功することがある。終了コードだけでは GPU の健全性を判定できない。
- 再起動は実施していない。`umr` は未インストール、`devcoredump` は root 権限が必要で読めなかった。AMF の内部バッファの寿命まで特定できていない。

## 問題点

1. AMF の非 NV12 出力用 VideoConverter が、複数の変換を in-flight にすると GPUVM fault を起こす。AMF ランタイム側の欠陥と推測される。
2. AMF SDK v1.5.2 の `AMF_Video_Decode_API.md` はデコーダーの `Init` のフォーマットを全コーデックで NV12 に限定している。P010 / P012 / P016 / I420 の出力は公式仕様外の実装依存動作である。
3. AMF が変換失敗をリカバリしてフレームを返す場合があり、GPU の device reset がテスト結果に現れない。
4. 参照 GPU と AMF 1.5.3.0 で観測した結果であり、他の GPU / AMF バージョンでの挙動は不明である。
5. 原因の正確な箇所 (AMF のバッファ管理か、RADV / ドライバーか) は未特定である。

## pending にした理由

非 NV12 出力の同時変換が AMF ランタイムで GPUVM fault と device reset を起こし、利用者に安全に提供できないため。amf-rs 側のコード修正だけでは解決できず、AMF 側の修正、または in-flight の変換数を制限する設計と十分な検証が必要である。AMF の修正または安全な回避策が確認できるまで pending とする。再開時は、カーネルログを監視しながら、NV12 / I420 / P010 / P012 / P016 の変換テストを並列・直列の両方で再実行する。
