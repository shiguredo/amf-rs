# フレームデータとビットストリームを安全に読み書きする API を追加し、公開 API から `ffi` 型を排除する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/change-safe-frame-api
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`amf` モジュールの `Buffer` と `Plane` は `get_native() -> *mut c_void` と `get_size()` / `get_hpitch()` しか公開しておらず、フレームデータやビットストリームを `&[u8]` / `&mut [u8]` として安全に読み書きする API がない。利用者は `std::slice::from_raw_parts` を自分で書く必要があり、クレートの主目的であるフレームの読み書きが安全に達成できない。テストも同じ unsafe を書いている。

また、`Buffer::set_pts` / `set_size` と `Surface::get_pts` がなく、入力 PTS の設定や出力 PTS の取得ができない。`Surface::get_plane`、`Surface::convert`、`Context::alloc_surface` などの公開シグネチャは `#[doc(hidden)]` の `ffi` 型 (`AMF_PLANE_TYPE`、`AMF_MEMORY_TYPE`、`AMF_SURFACE_FORMAT`) を要求し、doc から辿れない。

## 現状

- `src/amf.rs` の `Buffer` と `Plane` は生ポインタとサイズ、ピッチだけを公開する。
- `Buffer::get_pts` はあるが `set_pts` と `set_size` がない。`Surface` は `set_pts` はあるが `get_pts` がない。
- `Surface::get_plane` は `sys::AMF_PLANE_TYPE` を、`Context::alloc_surface` は `sys::AMF_MEMORY_TYPE` と `sys::AMF_SURFACE_FORMAT` を公開シグネチャで受け取る。`sys` は非公開で、`ffi` は `#[doc(hidden)]` である。
- `amf_pts` は i64 の別名で、単位 (100ns) が型から分からない。
- テストの `buffer_to_slice` と各コピーヘルパーは、null とピッチの検証なしに unsafe なスライス生成と書き込みを行っている。

## 設計方針

- `Buffer::as_slice` / `as_mut_slice` と `Plane::as_slice` / `as_mut_slice` を追加する。null、サイズ、`hpitch`、`vpitch` を検証し、範囲外は `Err` または `None` にする。
- `Buffer::set_pts` / `set_size` と `Surface::get_pts` を追加する。
- `PlaneType`、`MemoryType`、`SurfaceFormat` の安全な公開 enum を定義し、`sys` への変換を内部に閉じる。破壊的変更になるため、必要な移行を `CHANGES.md` に書く。
- `amf_pts` はニュータイプにするか、少なくとも doc に 100ns 単位であることを明記する。
- 既存の `get_native` / `get_size` / `get_hpitch` は低レベル用途として残すが、doc に unsafe の責任と有効期間を書く。
- README とテストを新しい API に更新する。
- `Plane` の寿命 (別 issue) と共有ロック (別 issue) の設計と整合させる。

## 完了条件

- `Buffer` / `Plane` から安全に `&[u8]` / `&mut [u8]` を取得できる。
- `Buffer::set_pts` / `set_size` と `Surface::get_pts` が追加される。
- 公開 API のシグネチャから `#[doc(hidden)]` の `ffi` 型がなくなる。
- null / ピッチ / サイズの検証テストが追加される。
- README とテストが新しい API に追随する。
- `cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace` が通る。
- `CHANGES.md` の `## develop` に `[CHANGE]` を追記する。
