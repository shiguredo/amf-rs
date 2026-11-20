# `examples/amf-check` の `rust-version` と README のビルド要件を整備する

- Created: 2026-11-10
- Completed: 2026-11-20
- Branch: feature/fix-build-requirements
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`examples/amf-check/Cargo.toml` に `rust-version` がなく、ワークスペースの MSRV 1.95 と一致していない。また、README と `skills/shiguredo-amf/SKILL.md` のビルド要件に、bindgen が要求する libclang が含まれていない。利用者がビルド環境を揃えられない。

## 現状

- `examples/amf-check/Cargo.toml` に `rust-version = "1.95"` がない。
- README の「ビルド時: git」に libclang の記載がない。
- `SKILL.md` のビルド要件にも libclang の記載がない。
- CI と `.devcontainer/Dockerfile` には libclang が入っているが、README には反映されていない。
- ワークスペースの MSRV は `Cargo.toml` の `rust-version = "1.95"` で定義されている。

## 設計方針

- `examples/amf-check/Cargo.toml` に `rust-version = "1.95"` を追加する。
- README と `SKILL.md` のビルド要件に libclang を追加し、Ubuntu でのパッケージ名 (`libclang-dev` など) を書く。
- CI / `.devcontainer/Dockerfile` の libclang の導入方法と README の記載が一致していることを確認する。
- `cargo metadata` で example の `rust-version` が取得できることを確認する。
- ドキュメントのみの変更のため `CHANGES.md` は更新しない。

## 完了条件

- `examples/amf-check/Cargo.toml` に `rust-version = "1.95"` がある。
- README と `SKILL.md` のビルド要件に libclang が記載されている。
- CI / devcontainer の導入方法と README の記載が一致している。
- `cargo metadata` で example の `rust-version` が確認できる。

## 解決方法

`examples/amf-check/Cargo.toml` に `rust-version = "1.95"` を追加し、ワークスペースの MSRV と揃えた。README と `skills/shiguredo-amf/SKILL.md` の動作要件に、bindgen が利用する libclang と Ubuntu の `libclang-dev` パッケージを追記した。

リモートの AMD 実機で `cargo metadata` を実行し、`amf-check` と `shiguredo_amf` の `rust_version` が `1.95` であること、`cargo build --workspace` が成功することを確認した。CI と `.devcontainer/Dockerfile` の libclang の導入方法は README の記載と一致している。ドキュメントのみの変更のため `CHANGES.md` は更新していない。
