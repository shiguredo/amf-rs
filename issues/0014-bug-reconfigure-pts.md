# `reconfigure` のフレームレート変更で PTS が不連続・逆行しないようにする

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
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
