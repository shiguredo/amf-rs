# Encoder / Decoder で AMFContext を共有し、Context をハンドルより長生きさせる

- Created: 2026-11-11
- Completed: {YYYY-MM-DD}
- Branch: feature/change-shared-amf-context
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

AMF の `AMFContext` はデバイスとアロケーターを表すパイプライン単位のオブジェクトであり、同じパイプラインのコンポーネントは同じ Context に載せるのが想定されている。AMD の transcode サンプルも、1 つの Context に decoder / converter / encoder を作成し、前段の出力を次段に渡している。

一方、現在の `Encoder::new` と `Decoder::new` はそれぞれ別の Context を作成している。このため、Decoder が出力した Surface を Encoder にそのまま渡すゼロコピーのパイプラインを組めない。また、Context を Encoder / Decoder が所有しているため、Surface / Buffer が Encoder / Decoder より長生きすると、terminate 済みの Context にアクセスする危険がある。

Encoder / Decoder で 1 つの Context を共有し、Context がコンポーネントとハンドルより長生きするようにする。

## 現状

- `src/encode.rs` の `Encoder::new` と `src/decode.rs` の `Decoder::new` は、それぞれ `AmfLibrary::create_context()` で新しい Context を作成し、`Context::init_vulkan(std::ptr::null_mut())` を呼んでいる。
- `Encoder` / `Decoder` は Context を所有し、`Drop` で `Context::terminate()` を呼んでいる。
- `src/amf.rs` の `Surface` / `Buffer` / `Plane` / `PropertyStorage` は Context への参照やライフタイムを持たない。Encoder / Decoder を drop した後もハンドルを保持でき、Context の terminate 後にアクセスできる。
- AMD の transcode サンプルは 1 つの `m_pContext` に decoder / converter / encoder を作成している。
- API Reference の基本的なパイプライン手順も、Context を作成してその Context にコンポーネントを作成し、前段の出力を次段に渡す流れになっている。
- 複数の Context を作成すること自体は API 上可能である。ただし、コンポーネント間で Surface / Buffer を受け渡す場合は同じ Context に載せる必要がある。

## 設計方針

- `AmfLibrary` がプロセス共有の Context を 1 つだけ遅延初期化して保持する。`init_vulkan` は `AmfLibrary` のロック下で 1 回だけ実行する。
- `Encoder::new` / `Decoder::new` は共有 Context を使う。Context の所有権は `AmfLibrary` に移し、Encoder / Decoder の `Drop` から `Context::terminate()` を外す。
- 共有 Context は Encoder / Decoder / コンポーネント / Surface / Buffer より長生きさせる。`AmfLibrary` が保持する Context はプロセス終了まで terminate しない。
- すべての Surface / Buffer が同じ Context に載るため、Decoder の出力 Surface を Encoder に渡すゼロコピーの経路を提供できる。Encoder が受け取る Surface が別 Context のものではないことを、Context の同一性で確認する。
- 複数 GPU を明示的に使う場合は、共有 Context ではなく利用者が指定した Context を使う API を別途検討する。デバイス選択の設計と整合させる。
- `Plane` の親 Surface に対する寿命 (`Plane<'a>`) は Context の寿命とは別の問題なので、本 issue では扱わない。
- `init_vulkan` の並列初期化の既知問題とは独立に、共有 Context の初期化を 1 回にすることで高レベル API からの並列初期化を避ける。任意の Context を作成する経路の直列化は別 issue のままとする。

## 完了条件

- `Encoder` と `Decoder` が同じ Context を使う。`AmfLibrary` の Context 初期化は 1 回だけで、並列に Encoder / Decoder を作成しても `InitVulkan` が重複しない。
- Encoder / Decoder の `Drop` が共有 Context を terminate しない。Encoder / Decoder を drop した後も、その Context から確保した Surface / Buffer を安全に利用できる。
- Decoder が出力した Surface を Encoder に投入する経路が、Context の同一性を確認した上で動作する。別 Context の Surface / Buffer は安全な API から投入できない。
- 共有 Context を使った Encoder / Decoder のテストが通り、`cargo clippy --workspace --all-targets -- -D warnings` と `cargo test --workspace -- --test-threads=1` が通る。
- `CHANGES.md` の `## develop` に `[CHANGE]` を追記する。
- `skills/shiguredo-amf/SKILL.md` と必要な doc を更新する。
