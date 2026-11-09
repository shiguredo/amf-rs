# `Error` の status 表と `Display` を整理する

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/refactor-error-design
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`src/error.rs` は `AMF_RESULT` の enum と `STATUS_TABLE` で同じ情報を二重に管理している。`Display` は status 名を 1 行に 2 回出し、`new_custom` と `from_amf` で `function` の型が統一されていない。エラー表現の一貫性がなく、新しい status を追加したときの更新漏れも起きやすい。

## 現状

- `STATUS_TABLE` が `AMF_RESULT` の全 status と名前・説明を持ち、enum の variant と二重管理になっている。
- `Display` のフォーマットが `"{}() failed[status={:?}]: {}"` とメッセージ内の `({name})` で status 名を 2 回出している。
- `new_custom` と `from_amf` で `function` が `Option<String>` と `&'static str` など異なる型になっている。
- `AMF_OK` の行は `Error::check` 経由では到達しない。
- `Error` の各 variant の doc が不足している。

## 設計方針

- status の名前と説明の情報源を 1 つにする。enum から名前を導出するか、表から enum を生成するかのどちらかに寄せる。
- `Display` は status 名を 1 回だけ出し、`function` とメッセージが読みやすい順に並べる。
- `new_custom` と `from_amf` の `function` の型を統一する。
- `AMF_OK` を表から外す場合は、`from_amf` が `AMF_OK` を受け取らないことを doc に書く。残す場合は到達経路を明記する。
- `AMF_RESULT` の未知の値を保持できるようにし、`Unknown` に丸めて元の値を失わない。
- `Error` の各 variant に doc を付け、`Display` の出力を検証するテストを追加する。

## 完了条件

- status の情報源が 1 つになっている。
- `Display` が status 名を 2 回出さない。
- `function` の型が統一されている。
- 未知の `AMF_RESULT` の元の値が保持される。
- `Error` の doc が追加され、`Display` のテストが通る。
- `CHANGES.md` の `## develop` の `### misc` にリファクタリングとして追記する。
