# `reconfigure` のフレームレート変更で PTS が不連続・逆行しないようにする

- Created: 2026-11-10
- Completed: 2026-11-20
- Branch: feature/fix-reconfigure-pts
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Encoder::encode` の PTS は `frame_count * framerate_den * AMF_SECOND / framerate_num` で毎回計算し直している。`Encoder::reconfigure` でフレームレートを変更すると、新しいフレームレートで全フレームの PTS を再計算した値になるため、PTS が不連続になる。30/1 で 5 フレーム投入した後に 60/1 へ変更すると 6 フレーム目の PTS は直前より小さくなり、30/1 から 15/1 へ変更すると約 2 秒ジャンプする。

既存のテストは 30/1 から 15/1 への変更だけを扱っており、PTS 自体も検証していない。

## 現状

- `src/encode.rs` の `Encoder::encode` が `frame_count * framerate_den * AMF_SECOND / framerate_num` で PTS を計算する。
- `Encoder::reconfigure` が `framerate_num` / `framerate_den` を更新する。
- PTS の計算は未検査の `u64` 乗算で、`framerate_den` に上限がないため、極端な設定ではオーバーフローする。
- 出力バッファの PTS を検証するテストがない。

## 設計方針

- PTS を前フレームからの差分で累積する。`pts += den * AMF_SECOND / num` を保持し、端数は剰余で繰り越して単調増加を保証する。
- `reconfigure` 時は「直前の PTS + 新しいフレーム間隔」で再基準化し、タイムラインを連続させる。
- `checked_mul` または `u128` でオーバーフローを防ぎ、`framerate_den` に妥当な上限を設ける。
- 30/1 から 60/1、30/1 から 15/1 の両方向のテストを追加し、出力 `Buffer::get_pts` の単調性を検証する。
- `Encoder::reconfigure` の doc に PTS の扱いを書く。

## 完了条件

- フレームレートの変更前後で PTS が単調増加し、不連続なジャンプがない。
- 極端なフレームレートでもオーバーフローしない。
- 30/1 から 60/1、30/1 から 15/1 のテストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。

## 解決方法

B フレーム対応で入出力の対応付けを `ShiguredoAmfFrameId` プロパティに変更した際に、`Encoder::encode` が投入フレーム数から PTS を計算して入力 `Surface` に設定する処理と、出力 `Buffer` の PTS を設定する処理を削除した。現在の `Encoder` は入力 `Surface` の PTS を変更せず、出力 `Buffer` の PTS も設定しない。`Encoder::reconfigure` は AMF のフレームレートプロパティを設定するだけで、PTS の状態を持たない。そのため、フレームレート変更による PTS の再計算、不連続、逆行、および未検査の乗算によるオーバーフローは発生しない。対象となる実装が存在しないため、対応不要として closed にする。
