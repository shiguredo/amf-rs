# `AmfLibrary` の二重遅延を整理し、`init_vulkan` の参照リークを修正する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-amf-library-design
- Polished: {YYYY-MM-DD}
- Refreshed: 2026-11-18
- Reporter: @melpon

## 目的

`Context::init_vulkan` は `AMFContext1` の `InitVulkan` vtable が取得できない場合に `?` で早期リターンするが、その時点で保持している `Context1` の参照を解放していない。`AMFContext1` は `QueryInterface` で参照カウントが増えているため、参照カウントがリークする。

`AmfLibrary` の `Mutex<Option<AmfLibraryInner>>` は #11 で削除され、`inner` は共有ロック下の `UnsafeCell<Option<AmfLibraryInner>>` になった。共有ロックを使う理由もコメントに書かれている。初期化に失敗した場合に再試行できるかは doc に書かれていない。

## 現状

- `src/lib.rs` の `AMF_LIBRARY` は `LazyLock<AmfLibrary>` で `AmfLibrary` を作る。`AmfLibrary::new` は AMF にアクセスせず、AMF ランタイムのロードは `ensure_inner` が初回呼び出し時に行う。
- `AmfLibrary` は `state: Arc<AmfObjectState>` と `inner: UnsafeCell<Option<AmfLibraryInner>>` を持ち、`ensure_inner` が `state` のロック保持中に `inner` を遅延初期化する。
- `AmfLibrary` の `state` と `inner` のフィールドコメントに、共有ロックの目的と `inner` へのアクセス条件が書かれている。`AmfLibrary::instance` と `ensure_inner` の doc には、AMF ランタイムが初回呼び出し時にロードされることが書かれている。
- `ensure_inner` は初期化に成功したときだけ `Some` を設定する。そのため初期化に失敗した場合は次の呼び出しで再試行されるが、この方針は doc に明記されていない。
- `src/amf.rs` の `Context::init_vulkan` が `QueryInterface` で `Context1` を取得した後、`InitVulkan` の vtable 取得に失敗すると `?` で抜け、`Context1` を `Release` しない。
- `init_vulkan` の直列化は issues/closed/0009 で実装済みで、`AmfLibrary` と `Context` は同じ `AmfObjectState` を共有し、`Context::init_vulkan` はそのロックを取る。
- `Context` の参照カウントは `tests/test_amf.rs` の `test_context_clone_and_handles_increment_ref_count` で検証されている。ただし検証しているのは `AMFContext` の成功経路で、`init_vulkan` の vtable 欠落経路は検証していない。

## 設計方針

- 初期化の失敗をキャッシュせず、次の呼び出しで再試行する方針を `ensure_inner` の doc に書く。
- `Context1` の取得後に早期リターンする経路でも `Release` されるようにする。RAII のガードを使うか、`?` の前に明示的に解放する。
- `init_vulkan` の成功・失敗の両方で参照カウントが釣り合うことをテストする。成功経路は既存の参照カウントテストで検証済みで、vtable 欠落経路のテストが未実施である。

## 完了条件

- 共有ロックの目的と、初期化失敗時に再試行する方針が doc とコメントで説明されている (AMF ランタイムのロードは既に `ensure_inner` の 1 か所にまとまっている)。
- `init_vulkan` の全経路で `Context1` の参照カウントが釣り合う。
- `init_vulkan` の vtable 欠落経路の参照カウントを検証するテストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。
