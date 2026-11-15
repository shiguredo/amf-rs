# `Plane` の寿命を `Surface` に縛り、`Surface` 解放後の無効な `Plane` を防ぐ

- Created: 2026-11-10
- Completed: 2026-11-15
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

## 解決方法

- `amf::Plane<'a>` が取得元の `&'a Surface` を保持するようにした。`Surface::get_plane` / `Surface::get_plane_at` は `Plane<'_>` を返し、`Plane` が生存している間は `Surface` を移動・破棄できない。
- `Plane::from_raw` / `Plane::from_raw_acquired` は `Context` ではなく `&Surface` を受け取り、取得元の `Surface` と状態を共有するようにした。
- `Plane::get_native` の doc に、返されるポインタの有効期間が `Surface` に依存することを明記した。
- 寿命と `Send` / `Sync` は `Plane<'a>` の型と impl から自明なため、`Plane` の公開 doc には重複して書かず、Safety コメントに根拠を残した。
- `Plane::from_raw` / `from_raw_acquired` の Safety 条件を、null、所有権、`as_ptr` の借用、並行 `convert` の観点で明確化した。
- `Plane::get_width` / `Plane::get_height` の doc を crop region の仕様に合わせた。
- `Plane` に `Debug` を実装した。
- `Plane` の `Send` / `Sync` は維持した。`Surface` が共有ロックで `Send + Sync` になっており、
  `Plane` も同じロックを共有するため、`Plane` から `Send` を外す必要がない。
- `Clone` / `Drop` / `context()` の参照カウント操作は、AMF Wiki の
  "The default implementation of AMFInterface is thread-safe." に基づき、共有ロックの対象外で
  あることを Safety コメントに明記した。
- `Surface` に Plane ハンドル数専用の `Arc<AtomicUsize>` を追加し、`Surface` の `Clone` 間で共有するようにした。`AmfObjectState` は変更せず、`Buffer` / `Context` / `Component` / `PropertyStorage` には影響しない。
- `Plane` の生成・`Clone` で `Surface` の Plane ハンドル数を増やし、drop で減らすようにした。
- `Surface::convert` は共有ロック内で Plane ハンドル数を確認し、1 個でも生存していればエラーを返すようにした。`Surface` の `Clone` や `PropertyStorage` が存在するだけでは失敗しない。
- `Surface::convert` の戻り値を `Result<(), Error>` に変更し、`src/decode.rs` の `extract_frame` を新しい戻り値に追随させた。
- `tests/test_amf.rs` で、`Plane` が 1 個でも生存していれば `Surface::convert` がエラーになること、
  `Surface` の `Clone` や `Plane` の `Clone` と Plane 数が共有されること、`get_plane_at` で取得した
  `Plane` でもエラーになること、`Plane` の drop 後は成功することを確認するテストを追加した。
- `PropertyStorage` が存在するだけでは `convert` が失敗しないことを確認するテストを追加した。
- `test_convert_allows_surface_clone` で、`Surface` の `Clone` だけでは `convert` が失敗しないことも
  確認するようにした。
- `Plane` を保持したまま `Surface` を `Encoder::encode` に投入するテスト (`tests/test_roundtrip.rs` の `test_encode_rejects_surface_with_live_plane`) は、コンパイル時に `Surface` の移動が拒否されるようになったため削除した。
- `src/amf.rs` の単体テストで、`Plane` の取得・`Clone` が `Surface` の状態を共有することと、drop すると共有状態が解消されることを確認するようにした。
- `skills/shiguredo-amf/SKILL.md` の `Plane` の API 説明を更新した。
- `CHANGES.md` の `## develop` に `[CHANGE]` を追記した。
- AMD GPU 実機で `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`DOCS_RS=1 cargo doc --no-deps` が通ることを確認した。
