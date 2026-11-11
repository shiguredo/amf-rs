# `amf` モジュールの AMF オブジェクトハンドルを共有ロックで保護し、`Send` と `Clone` を健全にする

- Created: 2026-11-10
- Completed: 2026-11-11
- Branch: feature/fix-amf-handle-thread-safety
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

AMF API Reference は `AMFPropertyStorage`、`AMFData`、`AMFSurface` を thread-safe ではないと明記している。thread-safe と明記されているのは `AMFComponent` だけである。一方、`amf` モジュールのラッパーは `unsafe impl Send` と安全な `Clone` を提供しており、`Clone` したハンドルを別スレッドに移すと safe なコードで同一の AMF オブジェクトに並行アクセスできる。これはデータ競合や SIGSEGV の原因になる。

AMF の参照カウント (`Acquire` / `Release`) はオブジェクトの寿命を管理するものであり、並行アクセスの安全性を保証するものではない。参照カウントを活かすために `Clone` は維持し、共有ロックによって `Clone + Send + Sync` を健全に成立させる。

## 現状

- `src/amf.rs` の `PropertyStorage`、`Surface`、`Buffer`、`Plane`、`Context` は `unsafe impl Send` と、内部で `Acquire` を呼ぶ `Clone` を持つ。
- `Component` だけは AMF の仕様で thread-safe と明記されている。
- `Surface::property_storage` / `Component::property_storage` が返す `PropertyStorage` は、親のロックを共有していない。
- `EncodeHandler::on_encoded` / `DecodeHandler::on_decoded` はワーカースレッドから呼ばれる。オブジェクトのロックを保持したままハンドラーを呼ぶと、ハンドラー内の同じオブジェクトへのアクセスで再入デッドロックする。
- `PropertyStorage`、`Surface`、`Buffer`、`Plane`、`Context` の `Clone` はクレート内では未使用である。`Component` の `Clone` だけがワーカースレッドへの受け渡しに使われている。
- `skills/shiguredo-amf/SKILL.md` は「すべて `unsafe impl Send` 済み (AMF オブジェクトはスレッドセーフであると AMF ドキュメントに記載されているため)」と書いているが、これは AMF の仕様と一致しない。

## 設計方針

- 各ラッパーに共有ロック (`Arc<Mutex<()>>` 相当) を持たせる。`Clone` は AMF の `Acquire` を呼び、ロックを `Arc::clone` で共有する。
- すべての FFI 呼び出しは 1 回ごとにロックを取得する。ロックを保持したままハンドラーなど外部コードを呼ばない。
- `Plane` と `PropertyStorage` は親 (`Surface` / `Component`) のロックを共有する。`Plane` の寿命の扱いは別 issue とする。
- `Component` は AMF の仕様で thread-safe なのでロック対象外とする。これは恣意的な例外ではなく、AMF の一次資料に基づく唯一の例外とする。
- `Context` も `Surface` などと同じ方針 (共有ロック + `Send + Sync`) にする。
- 内部の非公開ヘルパーはロック保持を前提とし、公開メソッド同士の二重ロックを避ける。
- ロックの取得順序は「親 → 子」に固定し、デッドロックを防ぐ。
- `unsafe impl Send` / `Sync` の根拠をコメントで明記し、`///` に `Clone` と並行利用の契約を書く。
- `skills/shiguredo-amf/SKILL.md` の thread-safe の記述を「`Component` のみ」に修正する。

## 完了条件

- `PropertyStorage`、`Surface`、`Buffer`、`Plane`、`Context`、`Component` が `Clone + Send + Sync` になる。`Send + Sync` の静的表明をテストに含める。
- `Clone` したハンドルを別スレッドで並行に使っても、ロックで直列化され、データ競合が起きない。
- ハンドラー呼び出しの前にオブジェクトのロックが解放される。
- 並行アクセスのテスト (`tests/test_amf.rs` など) を追加する。
- `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通る。
- `skills/shiguredo-amf/SKILL.md` の記述が実装と一致する。
- `CHANGES.md` の `## develop` に変更を追記する。

## 解決方法

- `src/amf.rs` に `AmfObjectState` を追加し、同じ AMF オブジェクトを指すハンドル間で `Mutex` とハンドル数を共有するようにした。`Clone` と子ハンドル (`Plane` / `PropertyStorage`) は状態を共有し、`from_raw` / `from_raw_acquired` で作成したハンドルは独立した状態を持つ。
- スレッドセーフでない AMF 呼び出しは `AmfObjectState` のロックで直列化した。ハンドラー呼び出しはロックを解放してから行う。
- `Component` は AMF の仕様で thread-safe なため自身のメソッドはロックせず、`PropertyStorage` のプロパティ操作だけを同じ取得元のハンドル間で直列化した。
- `Encoder::encode` / `Decoder::decode` で、投入する `Surface` / `Buffer` が他のハンドルと共有されていないことを確認し、共有されている場合はエラーを返すようにした。
- `src/amf.rs` の単体テストと `tests/test_amf.rs` に共有ロック、ハンドル数、`from_raw_acquired`、並行アクセスのテストを追加し、公開 doc と `skills/shiguredo-amf/SKILL.md` を実装に合わせて更新した。
