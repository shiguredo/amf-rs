# `Plane` の寿命を `Surface` に縛り、`Surface` 解放後の無効な `Plane` を防ぐ

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-plane-lifetime
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`Surface::get_plane` / `Surface::get_plane_at` が返す `Plane` は、AMF の `Acquire` によって独立した所有権を持つ。`Plane` は `Surface` への参照や `Arc` を保持していないため、`Surface` を drop した後も safe なコードで保持できる。AMF の仕様は「プレーンを `Surface` の外に保持するな」と明記しており、現在の API はこの契約に反する。

実機では、`Surface` を drop した後に `Plane` を使うと `get_hpitch` が 0、`get_vpitch` が 0、`get_native` が null になった。safe な API が無効な値を返し、利用者が `get_native` をそのまま書き込みに使うとクラッシュする。

## 現状

- `src/amf.rs` の `Plane` は `Clone` と `Send` を持ち、`Surface` の寿命に縛られていない。
- `Surface::get_plane` / `Surface::get_plane_at` は `Plane::from_raw_acquired` を返す。
- `Plane::get_native` / `get_hpitch` / `get_vpitch` は `Surface` の解放後も呼べる。
- 共有ロックを導入する場合、`Plane` は `Surface` のロックを共有する必要があるが、現状は共有していない。

## 設計方針

- `Plane<'a>` にして `&'a Surface` を保持させる、または `Surface` の `Arc` を保持させて `Surface` の寿命を延ばす。
- `Plane` から `Send` を外し、`Surface` と同じスレッドに縛る (共有ロックを導入する場合は親のロックを共有する)。
- `Plane::get_native` / `get_hpitch` / `get_vpitch` の doc に、有効期間が `Surface` に依存することを明記する。
- `Surface` の drop 後に `Plane` を使えないことをコンパイル時のライフタイムで保証する。コンパイルエラーの例を doc に載せる。
- 共有ロックの設計 (別 issue) と整合する形にする。

## 完了条件

- `Plane` が `Surface` より長生きできないことをコンパイル時に保証する。
- `Plane` の有効期間と `Send` / `Sync` の契約が doc に書かれている。
- 既存の `Surface` / `Plane` の利用箇所 (`tests/test_roundtrip.rs` のコピーヘルパーなど) が新しい API に追随する。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に後方互換性のない変更として追記する。
