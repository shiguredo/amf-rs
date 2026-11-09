# 公開 API の命名と小さな設計を整理する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-naming-api
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

公開 API と内部 API に、意味が分かりにくい名前と小さな設計の不統一がある。利用者が API を理解しやすくし、実装の一貫性を保つ。

## 現状

- `AmfLibrary::query_version` がタプルを返しており、バージョンの意味が型から分からない。
- `Context::alloc_surface` などの引数名が `type_` になっており、何の型か分かりにくい。
- `crate::Error` と `Error` のインポートが混在している。
- `Plane` / `Context` / `Component` / `PropertyStorage` に `Debug` が実装されていない (`Surface` / `Buffer` にはある)。
- `EncodeOptions` は `Copy` なのに参照で受け取っている。
- `FrameFormat::frame_size` は奇数の width / height を切り捨てるが、そのことが doc に書かれていない。

## 設計方針

- `query_version` の戻り値を `AmfVersion` のような名前付きの型にする。
- `type_` を `memory_type` など、実際の意味が分かる名前にする。
- `crate::Error` と `Error` のインポートを統一する。
- `Plane` / `Context` / `Component` / `PropertyStorage` に `Debug` を実装する。生ポインタはアドレスだけを出す。
- `EncodeOptions` は `Copy` なので値で受け取る。
- `frame_size` は奇数の切り捨てを doc に書くか、奇数を受け取ったら `None` を返すようにする。どちらにするかを決めてテストする。
- 公開 API の変更は破壊的変更になるため、`CHANGES.md` に `[CHANGE]` として記載する。

## 完了条件

- `query_version` の戻り値が名前付きの型になっている。
- `type_` などの曖昧な引数名がなくなっている。
- `Error` のインポートが統一されている。
- `Plane` / `Context` / `Component` / `PropertyStorage` が `Debug` を実装している。
- `EncodeOptions` が値で受け取られている。
- `frame_size` の奇数寸法の扱いが doc とテストで一致する。
- `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[CHANGE]` を追記する。
