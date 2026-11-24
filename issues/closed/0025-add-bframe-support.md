# B フレームを有効にしたエンコーダーの `AMF_NEED_MORE_INPUT` と出力順の対応付けを修正する

- Created: 2026-11-10
- Completed: 2026-11-18
- Branch: feature/add-bframe-support
- Reporter: @melpon

## 目的

B フレームを有効にすると、AMF のエンコーダーは最初の出力を出すまでに複数の入力サンプルを必要とし、`SubmitInput` が `AMF_NEED_MORE_INPUT` を返す。対応前の `Encoder::encode` はこれをエラーとして扱うため、B フレームを有効にできなかった。

また、対応前のワーカーは `user_data` と出力を FIFO で 1 対 1 に対応付けていた。B フレームでは入力順と出力順が一致しないため、`user_data` が誤ったフレームに付く。AMF の仕様も入出力の対応を仮定することを禁じている。

## 対応前の状況

- `EncoderConfig` に B フレームの設定がなく、`AMF_VIDEO_ENCODER_B_PIC_PATTERN` を設定していなかった。
- `Encoder::encode` の `SubmitInput` リトライループは、`AMF_OK` / `AMF_INPUT_FULL` / `AMF_REPEAT` 以外をエラーにしていた。`AMF_NEED_MORE_INPUT` はここでエラーになっていた。
- `src/encode.rs` のワーカーは `pending` (user_data) と `output_buffer` を FIFO で対応付けていた。
- 入力の PTS は投入フレーム数から計算しており、B フレームの出力順を識別する情報としては使っていなかった。
- デコーダー側の B フレーム対応は別 issue で扱う。

## 設計方針

- `EncoderConfig` に `b_pic_pattern` を追加し、H.264 では `AMF_VIDEO_ENCODER_B_PIC_PATTERN` と `AMF_VIDEO_ENCODER_MAX_CONSECUTIVE_BPICTURES`、AV1 では `AMF_VIDEO_ENCODER_AV1_B_PIC_PATTERN` と `AMF_VIDEO_ENCODER_AV1_MAX_CONSECUTIVE_BPICTURES` を設定する。HEVC は B フレームを制御するプロパティがないため指定をエラーにする。
- AMF が B フレーム非対応と報告した GPU では、要求が黙って無視されないよう `Encoder::new` がエラーを返す。能力情報を取得できない場合は対応可否を決めず、AMF に任せる。
- `AMF_NEED_MORE_INPUT` は「入力を受け付けたが、まだ出力がない」として扱い、エラーにしない。
- 入力 `Surface` に対応付け用の一意な値 (frame ID) をカスタムプロパティとして設定し、AMF が同じフレームの出力 `Buffer` へ引き継ぐ。ワーカーはこの値で `user_data` を引き当てる。FIFO の 1 対 1 前提はやめる。
- 入力 `Surface` の PTS は変更せず、出力 `Buffer` の PTS もライブラリ側では設定しない。時刻が必要な場合は利用者が `user_data` などで管理する。
- 入力 1 つにつき出力 1 つを前提とし、対応が取れない入出力はエラーとして通知する。
- H.264 の B フレーム入りラウンドトリップ、HEVC の指定拒否、AV1 の B フレーム非対応 GPU での拒否をテストする。

## 完了条件

- B フレームを有効にしたエンコーダーが `AMF_NEED_MORE_INPUT` で停止しない。
- 入力順と出力順が異なっても、`user_data` が正しいフレームに付く。
- 全フレームが 1 回ずつ出力される (欠落・重複がない)。
- H.264 の B フレーム入りラウンドトリップテスト、HEVC で `b_pic_pattern` を指定できないことを確認するテスト、AV1 の B フレーム非対応 GPU でエラーになるテストが追加される。
- `EncoderConfig` と `encode()` の doc が実装と一致する。
- `CHANGES.md` の `## develop` に `[CHANGE]` と `[ADD]` を追記する。

## 解決方法

- `EncoderConfig::b_pic_pattern` を追加し、H.264 では `AMF_VIDEO_ENCODER_B_PIC_PATTERN` と `AMF_VIDEO_ENCODER_MAX_CONSECUTIVE_BPICTURES`、AV1 では `AMF_VIDEO_ENCODER_AV1_B_PIC_PATTERN` と `AMF_VIDEO_ENCODER_AV1_MAX_CONSECUTIVE_BPICTURES` を設定するようにした。
  - 値域は AMF に任せ、AMF が返すエラーをそのまま返す。H.264 の仕様値域は 0-3、AV1 は 0-127 だが、実機の AMF は H.264 で 4 以上も受理することがあるため、ライブラリ側では制限しない。
  - HEVC の AMF エンコーダーには B フレームを制御するプロパティが存在しないため、`b_pic_pattern` の指定をエラーにした。
  - AMF の `BFrames` / `AV1BFrames` ケーパビリティが `false` を返した場合は、要求が黙って無視されないよう `Encoder::new` がエラーを返すようにした。ケーパビリティが取得できない場合は対応可否を判定せず、AMF に任せる。
  - AV1 の非対応 GPU では AMF が警告ログだけを出して `Encoder::new` が成功することを実機で確認した。
- `Encoder::encode` の `SubmitInput` で `AMF_NEED_MORE_INPUT` を入力の受理として扱うようにした。
- 入力 `Surface` に対応付け用の一意な値 (`ShiguredoAmfFrameId`) をカスタムプロパティとして設定し、出力 `Buffer` へ引き継いだ。ワーカーはこのプロパティで `user_data` を引き当てる。
  - frame ID は 1 から始まる `i64` の連番とし、`set_property_int64` / `get_property_int64` でそのまま扱う。
  - B フレームでは AMF が出力を符号化順に返し、AMF が付ける PTS はフレームの識別に使えないことを実機で確認した。
  - カスタムプロパティが H.264 / HEVC / AV1 の出力 `Buffer` へ引き継がれることを実機で確認した。
  - FIFO の 1 対 1 対応をやめ、`HashMap<i64, UserData>` で入力と出力を対応付けるようにした。
- 入力 `Surface` の PTS は変更せず、出力 `Buffer` の PTS もライブラリ側では設定しない。AMF が付けた値のままにし、時刻が必要な場合は `user_data` などで利用者が管理する。
- プロパティの読み出しは `Buffer::get_property` を削除し、`Buffer::property_storage` から `PropertyStorage::get_property` / `get_property_int64` を使うようにした。
- H.264 で B フレームを有効にしたラウンドトリップテストを追加し、全フレームが 1 回ずつ出力されること、`user_data` が対応する入力フレームに付くこと、デコード結果の内容が一致することを実機で確認した。
  - HEVC は AMF のエンコーダーが B フレームを扱わないため、B フレーム入りストリームの代わりに `b_pic_pattern` の指定がエラーになることをテストした。
  - AV1 の B フレームは VCN5 以降の GPU が必要で、今回の実機 (Radeon 780M) では非対応のため、非対応 GPU でエラーになることをテストした。
- `README.md` と `skills/shiguredo-amf/SKILL.md` を実装に合わせて更新し、`CHANGES.md` の `## develop` に `[CHANGE]` と `[ADD]` を追記した。
- AMD GPU 実機で `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps` を通した。
