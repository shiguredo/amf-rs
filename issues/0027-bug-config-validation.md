# `EncoderConfig` / `DecoderConfig` の入力値を検証する

- Created: 2026-11-10
- Completed: 2026-11-18
- Branch: feature/fix-config-validation
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

CODEBASE.md は「ユーザーが渡す値は検証すること」と定めているが、`EncoderConfig` / `DecoderConfig` の多くの値が検証されていない。不正な値は AMF の `Init` で `AMF_INVALID_ARGUMENT` などの分かりにくいエラーになり、どのフィールドが原因か分からない。

## 現状

- `EncoderConfig` の `width` / `height` は 0 や奇数を検証していない。
- `framerate_num` / `framerate_den` は 0 を検証しておらず、`framerate_den` の上限もない。
- CBR で `target_kbps` を指定しない場合の扱いが曖昧である。
- QP の範囲 (H.264 は 0-51、AV1 は 1-255 など) を検証していない。
- 10bit と `FrameFormat` の組み合わせは別 issue で扱う。
- `DecoderConfig` の `width` / `height` も同様に検証していない。
- 入力検証のテストがない。

## 設計方針

- `Encoder::new` / `Decoder::new` の入口で、コーデックごとの制約を検証する。
- width / height は 0 と負値を拒否し、コーデックが奇数を許さない場合は奇数も拒否する。
- `framerate_num` / `framerate_den` は 0 を拒否し、`framerate_den` に上限を設ける。
- CBR で `target_kbps` がない場合はエラーにする。VBR / CQP では必須にしない。
- QP はコーデックごとの範囲を検証する。`qpb` と `qp` の両方に適用する。
- 10bit とフォーマットの組み合わせは、別 issue の対応結果に合わせて検証する。
- エラーは `Error` の専用 variant にし、どのフィールドが不正かをメッセージに含める。
- 入力検証のテストを追加する。GPU 不要で実行できるようにする。

## 完了条件

- 不正な `EncoderConfig` / `DecoderConfig` が `new` で明確なエラーになる。
- 境界値 (0、奇数、QP の下限・上限) のテストが追加される。
- エラーメッセージから原因のフィールドが分かる。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。

## 解決方法

対応しない。CODEBASE.md の「ユーザーが渡す値は検証すること」は、FFI 境界で未定義動作や panic を起こす値を止めるための規約であり、設定値の妥当性を `Encoder::new` / `Decoder::new` の手前で検査することを求めるものではない。FFI 境界で未定義動作や panic を起こす値の検証は引き続き行う。

設定値が AMF の要求を満たさない場合は、AMF の `Init` が返すエラーをそのまま返す。QP の範囲、CBR の `target_kbps`、`framerate_den` の上限、`DecoderConfig` の width / height などをフィールドごとに事前検証すると、AMF の制約を二重に管理することになり、AMF 側の仕様変更に追従できなくなる。そのため、これらの検証は追加しない。
