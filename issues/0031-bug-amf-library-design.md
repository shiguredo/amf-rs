# `AmfLibrary` の二重遅延を整理し、`init_vulkan` の参照リークを修正する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-amf-library-design
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`AmfLibrary` は `LazyLock<AmfLibrary>` と `Mutex<Option<AmfLibraryInner>>` の二重の遅延構造になっており、初期化の状態が分かりにくい。Mutex を使う理由もコメントされていない。

また、`Context::init_vulkan` は `AMFContext1` の `InitVulkan` vtable が取得できない場合に `?` で早期リターンするが、その時点で保持している `Context1` の参照を解放していない。`AMFContext1` は `Acquire` されているため、参照カウントがリークする。

## 現状

- `src/lib.rs` の `AmfLibrary::instance` が `LazyLock` で `AmfLibrary` を作り、その中の `Mutex<Option<AmfLibraryInner>>` がさらに遅延初期化する。
- `src/lib.rs` の `Mutex` に、ロックが必要な理由のコメントがない。
- `src/amf.rs` の `Context::init_vulkan` が `QueryInterface` で `Context1` を取得した後、`InitVulkan` の vtable 取得に失敗すると `?` で抜け、`Context1` を `Release` しない。
- `init_vulkan` の直列化は別 issue で扱う。

## 設計方針

- `LazyLock` と `Mutex<Option<...>>` のどちらかを残し、もう一方を削除する。初期化の失敗をキャッシュしない、または再試行できるかを決めて doc に書く。
- `Mutex` を使う理由 (AMF ランタイムのグローバル状態を直列化するため) をコメントに書く。
- `Context1` の取得後に早期リターンする経路でも `Release` されるようにする。RAII のガードを使うか、`?` の前に明示的に解放する。
- `init_vulkan` の成功・失敗の両方で参照カウントが釣り合うことをテストする。
- 別 issue の `init_vulkan` 直列化と整合させる。

## 完了条件

- `AmfLibrary` の初期化経路が 1 つになり、状態が doc とコメントで説明されている。
- `init_vulkan` の全経路で `Context1` の参照カウントが釣り合う。
- リークの回帰テスト、または参照カウントを検証するテストが追加される。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[FIX]` を追記する。
