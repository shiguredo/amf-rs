# Encoder / Decoder で AMFContext を共有し、Context をハンドルより長生きさせる

- Created: 2026-11-11
- Completed: 2026-11-14
- Branch: feature/change-shared-amf-context
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

AMF の `AMFContext` はデバイスとアロケーターを表すパイプライン単位のオブジェクトであり、同じパイプラインのコンポーネントは同じ Context に載せるのが想定されている。AMD の transcode サンプルも、1 つの Context に decoder / converter / encoder を作成し、前段の出力を次段に渡している。

一方、現在の `Encoder::new` と `Decoder::new` はそれぞれ別の Context を作成している。このため、Decoder が出力した Surface を Encoder にそのまま渡すゼロコピーのパイプラインを組めない。また、Context を Encoder / Decoder が所有しているため、Surface / Buffer が Encoder / Decoder より長生きすると、terminate 済みの Context にアクセスする危険がある。

Context はアプリケーションが任意の個数作成できるようにし、Encoder / Decoder には利用者が作成した
`Context` を渡す。同じ Context を Clone して渡せばゼロコピーのパイプラインを組め、別の Context を
渡せば独立したパイプラインを組める。Context はハンドルより長生きするようにする。

## 現状

- `src/lib.rs` の `AmfLibrary::create_context` は非公開で、`AMFFactory::CreateContext` を呼ぶだけである。Vulkan デバイスの初期化は呼び出し側が行っている。
- `src/encode.rs` の `Encoder::new` と `src/decode.rs` の `Decoder::new` は、それぞれ `AmfLibrary::create_context()` で新しい Context を作成し、`Context::init_vulkan(std::ptr::null_mut())` を呼んでいる。
- `Encoder` / `Decoder` は Context を所有し、`Drop` で `Context::terminate()` を呼んでいる。
- `src/amf.rs` の `Surface` / `Buffer` / `Plane` / `PropertyStorage` は Context への参照やライフタイムを持たない。Encoder / Decoder を drop した後もハンドルを保持でき、Context の terminate 後にアクセスできる。
- `amf::Context` は公開型だが、公開されたコンストラクタを持たず、利用者が Context を作成できない。
- AMD の transcode サンプルは 1 つの `m_pContext` に decoder / converter / encoder を作成している。
- API Reference の基本的なパイプライン手順も、Context を作成してその Context にコンポーネントを作成し、前段の出力を次段に渡す流れになっている。
- 複数の Context を作成すること自体は API 上可能である。ただし、コンポーネント間で Surface / Buffer を受け渡す場合は同じ Context に載せる必要がある。

## 設計方針

- `AmfLibrary::create_context() -> Result<Context, Error>` を公開する。呼び出しごとに新しい `AMFContext` を作成し、既定の Vulkan デバイスで初期化する。プロセス共有の Context は持たない。`AmfLibrary` が保持するのは AMF ランタイムとファクトリだけとする。
- `amf::Context` は `AMFContext` の参照カウントを直接管理する Clone 可能なハンドルとする。
  `Clone` は `AMFContext::Acquire`、`Drop` は `AMFContext::Release` を呼ぶ。AMFContext の
  デストラクタが `Terminate` を呼ぶため、`Context` の `Drop` では `Terminate` を呼ばない。
- `Encoder::new` / `Decoder::new` は `Context` を受け取り、内部に保持する。同じ `Context` を Clone して渡せば同じ Context、別の `Context` を渡せば別の Context になる。
- AMFContext 自身の参照カウントを使うため、Rust 側の `Arc<Context>` は使わない。`Context` の
  Clone と Drop が AMF の参照カウントを増減させ、最後の参照が解放された時点で AMFContext が
  破棄される。
- `Surface` / `Buffer` / `Plane` / `PropertyStorage` / `Component` は `Context` の Clone を保持する。
  これにより、ハンドルが生きている限り Context は破棄されない。`Encoder` / `Decoder` を drop した
  後や、利用者が保持している `Context` をすべて drop した後でも、ハンドルが残っていれば Context は
  有効である。
- `Context` は `PartialEq` / `Eq` を実装し、同じ `AMFContext` を指すかを比較できる。`Surface` / `Buffer` は確保元の `Context` を保持し、`Encoder::encode` / `Decoder::decode` は自分と同じ Context のハンドルだけを受け取る。別 Context のハンドルはエラーにする。
- `Context::alloc_surface` / `alloc_buffer` は `&self` を受け取り、返すハンドルに `Context` の Clone を保持させる。`Encoder::alloc_surface` / `Decoder::alloc_buffer` も同じ仕組みを使う。
- `Surface` / `Buffer` / `Plane` / `PropertyStorage` / `Component` / `Encoder` / `Decoder` は
  `context() -> Context` を公開し、保持する Context を取得できるようにする。`is_from_context` の
  ような型ごとの専用メソッドは持たず、`Context` の `PartialEq` / `Eq` で比較する。
- `Context::terminate()` と `Context::into_raw()` は公開 API から削除する。Context の終了は最後の参照の drop に一本化し、利用者が任意のタイミングで終了させない。
- `Surface` / `Buffer` / `Plane` / `PropertyStorage` の `from_raw` / `from_raw_acquired` は
  `Context` を受け取る。生ポインタを再構築する場合も、どの Context のハンドルかを
  呼び出し元が指定する。`from_raw` は unsafe のままとし、ポインタと Context の対応は
  呼び出し元の責任とする。
- `Plane` の親 Surface に対する寿命 (`Plane<'a>`) は Context の寿命とは別の問題なので、本 issue では扱わない。
- 複数 GPU を明示的に使う場合は、`AmfLibrary::create_context()` に Vulkan デバイスを渡す API を別途検討する。デバイス選択の設計と整合させる。

## 完了条件

- `AmfLibrary::create_context()` で任意の個数の `Context` を作成できる。同じ `Context` を Clone して Encoder と Decoder に渡すことも、別の `Context` を渡すこともできる。
- Encoder / Decoder を drop した後も、その Context から確保した Surface / Buffer を安全に利用できる。利用者が保持している `Context` をすべて drop した後でも、ハンドルが残っていれば利用できる。
- Decoder が出力した Surface を同じ Context の Encoder に投入する経路が動作する。別 Context の Surface / Buffer は安全な API から投入できない。
- 複数の Context を並列に作成・初期化しても問題なく動作する。
- 同じ Context を共有する Encoder / Decoder と、別々の Context を使う Encoder / Decoder のテストが通り、`cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace -- --test-threads=1` が通る。
- `CHANGES.md` の `## develop` に `[CHANGE]` を追記する。
- `skills/shiguredo-amf/SKILL.md` と必要な doc を更新する。

## 解決方法

- `AmfLibrary::create_context()` を公開し、利用者が `AMFContext` を作成できるようにした。
- `amf::Context` を `AMFContext` の参照カウントを直接管理する `Clone` 可能なハンドルにした。`Clone` は `Acquire`、`Drop` は `Release` を行い、最後の参照の解放時に AMFContext のデストラクタが `Terminate` を呼ぶ。
- `Encoder::new` / `Decoder::new` が利用者の `Context` を第 1 引数で受け取り、内部で Context を生成しないようにした。
- `Surface` / `Buffer` / `Plane` / `PropertyStorage` / `Component` は `Context` の Clone を保持し、`from_raw` / `from_raw_acquired` は `Context` を受け取るようにした。
- `Context` に `PartialEq` / `Eq` を実装し、`Context` を保持する各ハンドルに `context()` を追加した。
- `Context::terminate()` / `Context::into_raw()` を削除した。
- `Encoder::encode` / `Decoder::decode` は別 Context の `Surface` / `Buffer` を拒否するようにした。
- `Context::Drop` の `Release` と `Terminate` はプロセス共有の `state` ロック内で実行し、AMF 呼び出しと直列化するようにした。
- AMD GPU 実機で `cargo fmt --all --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace -- --test-threads=1`、`DOCS_RS=1 cargo doc --no-deps` を通した。
