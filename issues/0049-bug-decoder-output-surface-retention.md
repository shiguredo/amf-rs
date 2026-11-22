# デコーダーワーカーが出力 GPU サーフェスを Host 変換せず保持するのを修正する

- Created: 2026-11-21
- Completed: 2026-11-22
- Branch: feature/fix-decoder-output-surface-retention
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

デコーダーのワーカーが `QueryOutput` で取得した出力 GPU サーフェスを Host メモリへ変換せずに `output_buffer` に保持している。出力が多いとデコーダーの出力サーフェスプールを使い切り、`AMF_DECODER_NO_FREE_SURFACES` によるデコード停止や、非 NV12 出力時の AMF VideoConverter の GPUVM fault を誘発する。ワーカーが GPU サーフェスを必要以上に保持しないようにし、出力形式にかかわらず安定してデコードできるようにする。

## 現状

- `src/worker.rs` の `drain_output` は `AMFComponent::QueryOutput` で取得した `Data` を `output_buffer` に積む。この時点では Host 変換しない。
- `src/worker.rs` の `deliver_buffered` が `WorkerConfig::deliver` を呼び、`src/decode.rs` の `Decoder::new` が渡すクロージャが `data.to_surface()` と `surface.convert(MemoryType::Host)` を行う。
- そのため、出力がまとめて得られると、Host 変換されるまで複数の GPU サーフェスが `output_buffer` に保持される。AMF のデコーダーの出力サーフェスプールは DPB + 5 程度で、アプリが出力を保持している場合と重なると枯渇する。
- NV12 出力では `AMF_DECODER_NO_FREE_SURFACES` が返り、デコードが停止する。非 NV12 出力では、同じサーフェス圧力が AMF の VideoConverter の GPUVM fault と device reset を誘発する。
- 実機 (AMD Radeon 780M / AMF ランタイム 1.5.3.0) の確認:
  - C の最小再現で、1080p I420 10 フレームを Host 変換せず保持すると 20 回中 3 回 page fault。Host 変換ありは 0/20。
  - NV12 で `SurfacePoolSize=1` を指定すると `AMF_DECODER_NO_FREE_SURFACES` が返り、GPU page fault は発生しない。
  - Rust の 10 フレーム H.264 I420 テストは 20 回中 7 回異常 (SIGSEGV 4、page fault 付き成功 3)。`drain_output` で即 Host 変換すると 20/20 成功、page fault 0。一時テストを含む `test_decode` 全体でも 5/5 成功、page fault 0。
- 非 NV12 出力そのものの AMF 仕様外動作は、この issue のスコープ外とする。この issue は、ワーカーが GPU サーフェスを保持する期間を短くする。

## 設計方針

- デコーダーの出力は、`output_buffer` に積む前に Host メモリへ変換する。`drain_output` 内で `Data::to_surface()` と `Surface::convert(MemoryType::Host)` を行い、`output_buffer` には Host サーフェスの `Data` を保持する。
- エンコーダーの出力は Host 変換の対象外とし、従来の経路を変えない。`WorkerConfig` にデコーダー用の出力準備処理を持たせるか、役割に応じた分岐を明示する。
- `Decoder::new` の deliver クロージャで行っている Host 変換は、ワーカー側へ移動する。二重変換を避け、`DecodedFrame::format()` は変換後のサーフェスから取得する。
- frame ID プロパティの読み取りは従来どおり行う。変換でプロパティが失われないことを確認する。
- 変換に失敗した場合は、出力を握りつぶさず `Err` としてハンドラーへ通知し、参照を解放する。
- テスト:
  - 出力を保持してサーフェスプールを枯渇させる NV12 の回帰テストを追加する。既定のプールサイズでは再現が不安定な場合は、テスト用に `AMF_VIDEO_DECODER_SURFACE_POOL_SIZE` を設定する方法を検討する。モックは使わず、AMD GPU 実機で実行する。
  - 10 フレーム以上を連続投入し、出力を保持したままデコードできることを確認する。
  - 非 NV12 の回帰は、別 issue の出力フォーマット機能が戻った段階で確認する。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。

## 完了条件

- デコーダーのワーカーが `output_buffer` に GPU サーフェスを保持しない。
- エンコーダーの既存挙動が変わらない。
- NV12 で出力を保持する回帰テストが追加され、`AMF_DECODER_NO_FREE_SURFACES` で停止しない。
- AMD GPU 実機で `cargo test --workspace` が通り、カーネルログに GPUVM page fault と device reset が出ない。
- `CHANGES.md` の `## develop` に `[FIX]` が追記される。

## 解決方法

- ワーカーの `drain_output` で、`QueryOutput` から取得した出力が Surface の場合は `output_buffer` に積む前に Host メモリへ変換するようにした。エンコーダーの Buffer 出力は変換しない
- `Decoder::new` の `deliver` から Host 変換を削除し、変換をワーカー側に集約した
- 変換に失敗した出力は、取得済みの `Data` を解放したうえで `Err` としてハンドラーへ通知する
- 出力を保持したまま 30 フレームを連続投入し、`AMF_DECODER_NO_FREE_SURFACES` で停止せず、通知された `Surface` が Host メモリであることを確認する回帰テストを追加した
- 既定の出力サーフェスプールでは `AMF_DECODER_NO_FREE_SURFACES` を再現できなかったため、テスト用の `AMF_VIDEO_DECODER_SURFACE_POOL_SIZE` 設定は追加していない。プールサイズを小さくするとデコーダー自体が動作しなくなり、修正前後で差が出ないため
- AMD Radeon 780M / AMF ランタイム 1.5.3.0 で `cargo test --workspace` が成功し、カーネルログに GPUVM page fault と device reset が出ないことを確認した
- `CHANGES.md` の `## develop` に `[FIX]` を追記した
