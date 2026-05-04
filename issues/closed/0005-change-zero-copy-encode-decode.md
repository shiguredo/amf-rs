# 0005 - encode/decode の入出力を Surface/Buffer に変更してゼロコピー化する

Created: 2026-05-04
Completed: 2026-05-04
Model: deepseek-v4-pro

## 背景と根拠

現在の `encode()` / `decode()` は `&[u8]` を入力として受け取り、内部で
`Surface` / `Buffer` を確保してデータをコピーしている。
また、エンコード/デコードのコールバックでは `EncodedFrame` / `DecodedFrame` が
それぞれ `Vec<u8>` を保持しており、内部的に `to_vec()` による余分なコピーが発生している。

利用者がすでに `Surface` や `Buffer` を持っている場合でも `Vec<u8>` を経由する必要があり、
無駄なメモリコピーが発生している。

## 解決方法

### 1. encode() の入力: `&[u8]` → `Surface`

`Encoder::encode()` が `Surface` を直接受け取るように変更した。
内部での `alloc_surface` + `copy_frame_to_surface` を削除し、呼び出し元が用意した
`Surface` をそのまま AMF に送出する。

`Encoder::alloc_surface()` メソッドを追加し、利用者が簡単に Surface を確保できるようにした。

### 2. decode() の入力: `&[u8]` → `Buffer`

`Decoder::decode()` が `Buffer` を直接受け取るように変更した。
内部での `alloc_buffer` + データコピーを削除し、呼び出し元が用意した
`Buffer` をそのまま AMF に送出する。

`Decoder::alloc_buffer()` メソッドを追加した。

### 3. EncodedFrame の出力: `Vec<u8>` → `Buffer`

`EncodedFrame<T>` が `Buffer` を直接保持するように変更した。
出力抽出では `Vec<u8>` への `to_vec()` コピーを排除し、`Buffer` をそのまま返却する。

最終的な `EncodedFrame<T>` の定義:
```rust
pub struct EncodedFrame<T> {
    buffer: Buffer,
    picture_type: PictureType,
    user_data: T,
}
```
- `picture_type` はコーデック別のプロパティ名を指定して Buffer から取得する必要があり、ロジックが複雑なため EncodedFrame に保持する
- `pts` は `buffer.get_pts()` で取得可能なため、EncodedFrame では保持しない
- `data: Vec<u8>` および `data()` / `into_data()` アクセサは削除し、`buffer()` / `into_buffer()` に置き換え

### 4. DecodedFrame の出力: `Vec<u8>` → `Surface`

`DecodedFrame<T>` が `Surface` を直接保持するように変更した。
フレーム抽出では `convert(AMF_MEMORY_HOST)` のみ実行し、`Vec<u8>` へのプレーンデータコピーを排除する。

最終的な `DecodedFrame<T>` の定義:
```rust
pub struct DecodedFrame<T> {
    surface: Surface,
    user_data: T,
}
```
- `width` / `height` は `surface.get_plane(AMF_PLANE_Y)` 経由で取得可能なため保持しない
- `data: Vec<u8>` および `data()` / `into_data()` アクセサは削除し、`surface()` / `into_surface()` に置き換え

### 5. EncodedFrame / DecodedFrame に T を持たせる

`EncodedFrame<T>` / `DecodedFrame<T>` に `user_data: T` フィールドを追加し、
コールバックの引数から `T` を削除した。

- 変更前: `callback: FnMut(EncodedFrame, T)` / `callback: FnMut(DecodedFrame, T)`
- 変更後: `callback: FnMut(EncodedFrame<T>)` / `callback: FnMut(DecodedFrame<T>)`

### 6. copy_frame_to_surface の削除

`Encoder` の内部メソッドであった `copy_frame_to_surface` を完全に削除した。
フレームデータの Surface へのコピーは利用者側で行う。

### 7. Surface / Buffer への Debug 実装

`Surface`、`Buffer`、`SurfaceFuncs`、`BufferFuncs` に `#[derive(Debug)]` を追加した。

### 8. Encoder.frame_format の削除

`copy_frame_to_surface` 削除に伴い、`Encoder<T>` から未使用の `frame_format` フィールドを削除した。

### 9. positive_i32_to_usize の削除

`error.rs` の `positive_i32_to_usize` は `copy_frame_to_surface` と `extract_frame` の
簡素化により未使用となったため削除した。

### 10. テストの修正

- NV12/I420/YV12/Packed データを Surface にコピーするヘルパー関数を追加
- 全テストのコールバックシグネチャを更新
- `encode()` 呼び出し前に `alloc_surface()` + `copy_*_to_surface()` を実行
- `decode()` 呼び出し時に `frame.into_buffer()` で Buffer を渡す
- PSNR 計算時に `decoded.surface()` から Y プレーンを読み取り
- SPS/PPS 検証時に `frame.buffer()` から `buffer_to_slice()` でデータ取得
- ラウンドトリップ検証時に幅/高さを `surface.get_plane(Y)` から取得

### 11. README.md の更新

エンコード・デコードの使用例を新しい API に合わせて更新した。
PTS は `buffer.get_pts()`、幅/高さは `surface.get_plane(...)` 経由の例を記載。
