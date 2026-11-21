# エンコーダーから SPS / PPS / VPS を取得できるようにする

- Created: 2026-11-10
- Reporter: @melpon

## 目的

MP4 の avcC / hvcC / av1C や RTP の fmtp を組み立てるには、SPS / PPS / VPS などのパラメータセットが必要である。vpl-rs と nvcodec-rs は `Encoder::get_sequence_params()` を、video-toolbox-rs は `EncodedFrame` の `sps_list` / `pps_list` / `vps_list` を提供している。amf-rs はビットストリーム全体しか返さず、利用者が自前で解析する必要がある。

## 現状

- `src/encode.rs` の `EncodedFrame::buffer()` はビットストリーム全体の `Buffer` を返す。SPS / PPS / VPS を切り出す API はない。
- `src/encode.rs` は `AMF_VIDEO_ENCODER_EXTRADATA`、`AMF_VIDEO_ENCODER_HEVC_EXTRADATA`、`AMF_VIDEO_ENCODER_AV1_EXTRA_DATA` を読んでいない。
- `src/amf.rs` の `PropertyStorageFuncs` は `SetProperty` だけを保持し、`GetProperty` を持たない。`AMF_VARIANT_INTERFACE` から `AMFBuffer` を取り出すヘルパーもない。

## 検討した設計方針

以下は検討した案であり、実装は行っていない。

- `Encoder::get_sequence_params() -> Result<Option<SequenceParams>, Error>` を追加する。
- `SequenceParams` は `vps: Option<Vec<u8>>`、`sps: Vec<u8>`、`pps: Option<Vec<u8>>` を持つ。AV1 は `sps` に Sequence Header OBU を入れる (vpl-rs と同じ表現)。
- H.264 / HEVC の `EXTRADATA` は Annex B (スタートコード付き) で返るため、NAL ユニットに分割し、スタートコードを除去して格納する。H.264 は SPS / PPS、HEVC は VPS / SPS / PPS を NAL ユニット種別で判別する。
- AV1 の `AMF_VIDEO_ENCODER_AV1_EXTRA_DATA` は OBU のまま返す。OBU の分割は行わず、`sps` に格納する。
- `PropertyStorage` に `get_property_int64` などの取得ヘルパーを追加し、`AMF_VARIANT_INTERFACE` から `AMFBuffer` を取り出す。`GetProperty` が返すインターフェースの参照カウント (Acquire / Release) を確認し、二重解放を避ける。
- `EXTRADATA` が未設定、空、または取得できない場合は `Ok(None)` を返す。
- `encode()` と同じスレッドで呼ぶこと、`reconfigure` で内容が変わり得ることを doc に書く。
- 実機テストを追加する。H.264 / HEVC / AV1 で空でないパラメータセットが返ること、スタートコードが含まれないこと、`reconfigure` 後に内容が変わること、`finish()` 後でも取得できることを検証する。

## 完了条件

- `Encoder::get_sequence_params()` が公開され、`SequenceParams` を返す。
- H.264 / HEVC / AV1 で SPS / PPS / VPS または Sequence Header OBU が取得できる。
- スタートコードの扱いと AV1 の表現が doc に書かれている。
- 取得できない場合に `Ok(None)` を返す。
- 実機テストが追加され、`cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[ADD]` を追記する。

## 調査内容

- AMF の `AMF_VIDEO_ENCODER_EXTRADATA`、`AMF_VIDEO_ENCODER_HEVC_EXTRADATA`、`AMF_VIDEO_ENCODER_AV1_EXTRA_DATA` を実機で調査した。
- `Init` 直後から取得できるが、初期値は実際のストリームと一致しないことがある。H.264 は 25 バイト (SPS + PPS)、HEVC は 92 バイト (VPS + SPS + PPS)、AV1 は 15 バイト (Sequence Header OBU) だった。
- エンコード開始後と `reconfigure` 後に内容が変わる。`reconfigure` の変更は次の IDR の出力で反映される。
- `finish` の後は `GetProperty(EXTRADATA)` が `AMF_FAIL` になる。Drain 中も取得に失敗することがある。
- `GetProperty` は同じ `AMFBuffer` への参照を返す。`AMFVariantCopy` は同じ interface pointer を `Acquire` するだけで、新しい `AMFBuffer` を作るわけではない。
- AMF のドキュメントには `EXTRADATA` の更新タイミングの保証がない。AMF のサンプルも `EXTRADATA` を初期化時にしか読んでおらず、`TranscodePipeline.cpp` には `SetProperty(EXTRADATA, NULL)` で再生成させるというコメントアウトされた回避策がある。
- 試作では `PropertyStorage::get_property_buffer`、`BufferHandle`、`SequenceParams`、`Encoder::get_sequence_params`、`EncodedFrame::sequence_params` を実装し、ワーカースレッドが IDR の出力時に `EXTRADATA` を読んでキャッシュする方式を検証した。`finish` の Drain 前にキャッシュを更新するフォールバックも試した。
- H.264 / HEVC / AV1 で IDR の出力時に取得した値がビットストリームと一致すること、非 IDR フレームに直前の IDR の値が付くこと、`reconfigure` 後に値が変わること、出力を待たずに `finish` しても取得できること、IDR を連続投入しても取得できることを実機テストで確認した。AMD GPU の実機では全 154 テストが成功した。
- ただし、これは対象 GPU と AMF の実装で観測した結果であり、データ競合の不在や他の環境での保証ではない。
- 代替案として、IDR の出力ビットストリームから SPS / PPS / VPS / Sequence Header を抽出する方式を検討した。出力バッファはこちらが所有するため AMF 内部の更新と競合しない。ただし、H.264 の `HEADER_INSERTION_SPACING`、HEVC の `INSERT_HEADER`、AV1 の `FORCE_INSERT_SEQUENCE_HEADER` を IDR に合わせて設定し、NAL / OBU を解析する必要がある。`new` 直後を `None` にする API 変更も必要になる。

## 問題点

1. `EXTRADATA` の更新タイミングが AMF の仕様で保証されていない。実機で観測した挙動に依存している。
2. `GetProperty` が返す `AMFBuffer` は AMF 所有であり、IDR の更新中に AMF 内部が書き込む可能性がある。公開されているロックやスナップショット取得 API がない。
3. `Buffer` の `AmfObjectState` や `BufferHandle` のロックは、こちら側のハンドル間の操作を直列化するだけで、AMF 内部の書き込みは防げない。
4. Drain 中・後の取得失敗に対するフォールバックが必要になり、実装が複雑になる。
5. 出力時点の値をフレームに付けても、in-flight フレームや `reconfigure` が絡むと、そのフレームに対応する値であることを保証できない。
6. `new` 直後の値は暫定値であり、最初の IDR の値と一致しないことがある。
7. 実機テストは対象 GPU と AMF の実装で観測しただけで、データ競合の不在や他の GPU / AMF バージョンでの保証にはならない。
8. 安全な方式に切り替えるには、IDR 出力ビットストリームからの抽出が必要になる。そのためには IDR ごとのヘッダー挿入設定と NAL / OBU 解析が必要で、`new` 直後の値を `None` にするなどの API 変更も伴う。

## pending にした理由

AMF 所有の `EXTRADATA` を安全に読む方法がなく、公開 API として `get_sequence_params` を提供するにはビットストリーム抽出への設計変更が必要である。ビットストリーム抽出は IDR ごとのヘッダー挿入と NAL / OBU 解析を伴い、API の意味も変わるため、現時点では実装を進めない。設計判断が固まるまで pending とする。
