# 0004 - AMF オブジェクトの RAII ラッパー導入

Created: 2026-05-03
Completed: 2026-05-04
Model: deepseek-v4-pro

## 背景と根拠

現在の amf-rs は `sys::AMFSurface` や `sys::AMFComponent` などの生ポインタを直接利用している。
vtable の関数を呼ぶために以下のようなコードが必要で、非常に冗長になっている:

```rust
unsafe {
    let vtbl = &*(*surface).pVtbl;
    let f = require_vtbl_fn(vtbl.SetPts, "SetPts")?;
    f(surface, pts)
}
```

また、参照カウント管理 (`Acquire`/`Release`) を手動で呼ぶ必要があり、
`Drop` 実装やエラーパスでの解放漏れが発生しやすい状態だった。

AMF SDK のドキュメント (`../AMF/amf/public/include/core/Interface.h`) では
C++ 向けに `AMFInterfacePtr_T<T>` というスマートポインタが提供されており、
同様の RAII ラッパーを Rust 側でも実装することでこれらの問題を解決する。

本 issue では以下の課題を解決する:

1. `pVtbl` への直接アクセスによるボイラープレートの削減
2. 手動 `Acquire`/`Release` 管理からの脱却
3. `SendableComponent` のような場当たり的なラッパーの一掃

## 解決方法

### 1. `src/amf.rs` の新規作成

以下の RAII ラッパー型を実装 (`pub mod amf` として公開):

| ラッパー型 | 内部ポインタ | Clone | Drop | Send | 備考 |
|-----------|------------|-------|------|------|------|
| `Surface` | `*mut AMFSurface` | Acquire | Release | Yes | |
| `Buffer` | `*mut AMFBuffer` | Acquire | Release | Yes | |
| `Plane` | `*mut AMFPlane` | Acquire | Release | Yes | |
| `Context` | `*mut AMFContext` | Acquire | Release | Yes | |
| `Component` | `*mut AMFComponent` | Acquire | Release | Yes | |
| `PropertyStorage` | `*mut AMFPropertyStorage` | Acquire | Release | Yes | |

各ラッパー型は `from_raw` (呼び出し元の参照を引き継ぎ、Acquire なし) と `from_raw_acquired` (Acquire で参照を共有) の
2 種のコンストラクタを持つ。
`from_raw` は呼び出し元の参照を引き継ぎ (Acquire なし)、
`from_raw_acquired` は新たに参照を共有する (Acquire を呼ぶ)。
`alloc_surface` や `alloc_buffer` 等で生成されたオブジェクト (参照カウント 1) のラップには `from_raw` を使い、
キャストや参照カウントを増やさない関数を使う場合は `property_storage()` や `get_plane()` のように `from_raw_acquired` を使って参照カウントを増やす。

### 2. vtable 関数ポインタの事前取り出し

各ラッパー型は、コンストラクタ時に vtable から必要な関数ポインタをすべて取り出し、
専用の `*Funcs` 構造体に格納する。

```rust
struct SurfaceFuncs {
    acquire: unsafe extern "C" fn(...),
    release: unsafe extern "C" fn(...),
    set_pts: unsafe extern "C" fn(...),
    get_plane: unsafe extern "C" fn(...),
    // ...
}

pub struct Surface {
    ptr: *mut sys::AMFSurface,
    f: SurfaceFuncs,
}
```

これにより、各メソッド呼び出し時に vtable の `Option` チェック (`require_vtbl_fn`)
が不要になり、毎回 `Result` を返す必要がなくなる。
ランタイムエラー (`AMF_RESULT`) を返すメソッドのみが `AMF_RESULT` を返す。

### 3. PropertyStorage ラッパー

`Surface` と `Component` はどちらも `AMFPropertyStorage` を継承しているため、
共通の `PropertyStorage` ラッパーを経由してプロパティを設定できる。

AMF SDK の実装 (C/C++ サンプルおよび `AMF_INTERFACE_ENTRY` マクロ) では、
基底インタフェースへのアクセスにポインタキャストを使用している。
`AMFSurfaceVtbl` は `AMFPropertyStorageVtbl` の全メソッドを同一オフセットに含むため、
vtable ポインタを `*const AMFPropertyStorageVtbl` にキャストすることで
`Acquire`/`Release`/`SetProperty` を直接呼び出せる。

```rust
impl Surface {
    pub fn property_storage(&self) -> PropertyStorage {
        unsafe {
            PropertyStorage::from_raw_acquired(self.ptr as *mut sys::AMFPropertyStorage)
                .expect("...")
        }
    }
}
```

`PropertyStorage` には `set_property_int64`、`set_property_size`、`set_property_rate` の
コンビニエンスメソッドを実装し、`&str` のワイド文字列変換とエラーチェックを内包する。

### 4. extract_frame / extract_encoded_output の Result 化

ワーカースレッド内の出力抽出関数 (`extract_frame`, `extract_encoded_output`) は、
`from_raw_unchecked` ではなく `from_raw` を使い、`Result<T, Error>` を返すように変更。
呼び出し元の `drain_output` は `Result<(), Error>` を返し、`worker` はエラー時に
`log::error!` を出力して `continue` する。

### 5. Encoder / Decoder の Drop 順序

`Component` → `Context` の順で宣言し、`Drop::drop` では `terminate()` のみを
明示的に呼び、`Release` は各フィールドの自動 Drop に任せる。

### 6. 既存コードの移行

- **`src/lib.rs`**: `AmfLibrary::create_context()` が `Result<Context, Error>` を返すように変更。
  `create_component()` が `Result<Component, Error>` を返すように変更。
  `init_vulkan` を `Context::init_vulkan` に移動。
  `mod amf` を `pub mod amf` に変更し、`pub use amf::{...}` で再エクスポート。

- **`src/encode.rs`**: `Encoder<T>` のフィールドを `Component` と `Context` に変更。
  `SendableComponent` を削除（`Component` が `Send` を実装するため）。
  `set_component_prop_int64` 等のヘルパー関数を削除し `PropertyStorage` で統一。
  全 vtable アクセスをラッパーメソッド呼び出しに置換。

- **`src/decode.rs`**: 同上。

- **`src/codec_info.rs`**: `ProbeContext` が `Context` ラッパーを直接保持するように変更。
  `try_create_component` を `lib.create_component(&self.context, id).is_ok()` に簡略化。

### 7. 削除されたコード

- `SendableComponent` (encode.rs, decode.rs)
- すべての `&*(*ptr).pVtbl` ボイラープレート
- すべての手動 `Acquire`/`Release` 呼び出し
- `release_buffer()` ヘルパー関数
- `set_component_prop_int64` / `set_component_prop_size` / `set_component_prop_rate` / `set_surface_prop_int64`
