# `codec_info` の能力情報を実機と一致させる

- Created: 2026-11-10
- Completed: {YYYY-MM-DD}
- Branch: feature/fix-codec-info
- Polished: {YYYY-MM-DD}
- Reporter: @melpon

## 目的

`supported_codecs` が返す `CodecInfo` の能力情報が、実機の AMF と無関係の固定値になっている。利用者はプロファイルや B フレーム対応を正しいものと信じて使うため、誤った情報を返さないようにする。

AMF が返した値を Rust の型に無理やり押し込めない。`AMFCaps` / `AMFIOCaps` が返す情報を、解像度、フォーマット、メモリタイプ、コンポーネント固有プロパティを含めてそのまま返す。値が存在しない場合は `None` で表し、未知の値は生値を保持する。

## 現状

- `EncodingInfo` の `profiles` はヘッダー定義をもとにした固定の一覧で、ハードウェアが未対応のプロファイルも含み得る。
- `supports_frame_reordering` は H.264 / HEVC が常に true、AV1 が常に false である。`supports_multi_pass` は常に false である。
- `hardware_accelerated` は常に `supported` と同じ値で、情報量がない。
- コーデックの対応判定は `CreateComponent` の成否だけで、`Init` を試していない。
- `CodecInfo` には解像度、対応フォーマット、メモリタイプ、最大ビットレートなどの情報がない。
- 「AMF SDK にはプロファイル一覧を問い合わせる API がない」というコメントがあるが、`AMFComponent::GetCaps` と `AMFCaps` に `AMF_VIDEO_ENCODER_CAP_MAX_PROFILE`、`AMF_VIDEO_ENCODER_CAP_BFRAMES` などの能力照会がある。
- `AMFCaps` は `AMFPropertyStorage` を継承し、`GetAccelerationType`、`GetInputCaps`、`GetOutputCaps` と、コンポーネント固有のプロパティを持つ。
- `AMFIOCaps` は幅・高さの範囲、垂直アライン、surface format と memory type（それぞれ native フラグ付き）、インターレース対応を持つ。
- `AMF_SURFACE_FORMAT` / `AMF_MEMORY_TYPE` / `AMF_VARIANT_TYPE` は bindgen で rustified enum として生成されているため、未知の値を受けると Rust の enum として不正な値になる。
- 実機 (AMD Radeon 780M / AMF 1.5.3) で `GetCaps` を確認した。
  - 全コーデックで `CreateComponent` 直後の `Init` 前に `GetCaps` が成功し、`Init` 後も同じ値を返す。
  - H.264 エンコーダー: 20 プロパティ。`MaxProfile=100` (High)、`MaxLevel=52` (5.2)、`BFrames=true`、`MaxBitrate=1000000000`。入力は 128..4096 x 128..4096、垂直アライン 32、6 フォーマット、3 メモリタイプ。
  - HEVC エンコーダー: 17 プロパティ。`HevcMaxProfile=2` (Main10)、`HevcMaxLevel=186` (6.2)、B フレーム対応のプロパティは存在しない。入力は 128..8192 x 128..4352、7 フォーマット、3 メモリタイプ。
  - AV1 エンコーダー: 19 プロパティ。`Av1MaxProfile=1` (Main)、`Av1MaxLevel=16` (6.0)、`AV1BFrames=false`、`Av1WidthAlignmentFactor=64`、`Av1HeightAlignmentFactor=16`。入力は 128..8192 x 128..4352、7 フォーマット、3 メモリタイプ。
  - H.264 デコーダー: 3 プロパティ。入力は 32..4096 x 32..4096、出力は 4 フォーマット、2 メモリタイプ。
  - HEVC / AV1 デコーダー: 3 プロパティ。入力は 32..8192 x 32..4320、出力は 4〜5 フォーマット、2 メモリタイプ。
  - 全コンポーネントで `GetAccelerationType()=AMF_ACCEL_HARDWARE`。
  - 存在しないプロパティへの `GetProperty` は `AMF_NOT_FOUND` を返し、値は空のままになる。
- 実機の caps には、公開ヘッダーに定数がないプロパティ (`CapTransform8x8`、`EncoderCapNormalizedThroughput`、`AV1SupportedPictureManagementMethod` など) も含まれる。これらを落とさないため、`properties` で全件を返す。

## 設計方針

### AMF の能力照会

- `amf` モジュールに `Component::get_caps()` と、`Caps` / `IoCaps` の RAII ラッパーを追加する。0022 ではクレート内公開に留める。
- `ComponentFuncs` に `GetCaps` を追加する。古い AMF ランタイムで `GetCaps` が存在しない場合でもエンコーダー/デコーダー自体は使えるように、`GetCaps` は `Option` で保持し、呼び出し時に無ければエラーにする。
- `Caps` は `Release` を `Drop` で呼ぶ。`GetCaps` の呼び出しは `Component` の他の操作と同じプロセスロックで直列化する。
- `Caps` に次の機能を追加する。
  - `get_acceleration_type()`
  - `input_caps()` / `output_caps()`
  - `get_property_count()` / `get_property_at(index)`
  - `get_property_int64(name)` / `get_property_bool(name)`
- `IoCaps` に次の機能を追加する。
  - `width_range()` / `height_range()`
  - `vertical_alignment()`
  - `formats()`（`AMF_SURFACE_FORMAT` と native フラグ）
  - `memory_types()`（`AMF_MEMORY_TYPE` と native フラグ）
  - `interlaced()`
- `AMFIOCaps` が公開するアライメントは `GetVertAlign` の縦方向だけである。したがって `IoCaps.vertical_alignment` は `AMFIOCaps::GetVertAlign` の値（行数の倍数制約）を表し、横方向のアライメントは `IoCaps` には含めない。
- `GetPropertyAt` は `AMFVariantCopy` で値をコピーするため、`STRING` / `WSTRING` は AMF が確保した領域を `free` し、`INTERFACE` は `Release` する。Linux では AMF の variant 文字列が `malloc` で確保されるため `free` で解放できる。
- `GetPropertyAt`、`GetFormatAt`、`GetMemoryTypeAt` のいずれかが途中で失敗した場合は、その `Caps` / `IoCaps` の取得全体をエラーにする。取得できた分だけを返して情報を黙って落とさない。
- `GetCaps` が返した `AMFCaps` はコンポーネントを `Terminate` する前に破棄する。

### FFI の未知値

- `AMF_SURFACE_FORMAT` / `AMF_MEMORY_TYPE` / `AMF_VARIANT_TYPE` を bindgen の `newtype_enum` に変更する。rustified enum のままでは、AMF が将来追加した値を安全に保持できないためである。
- `AMF_RESULT` は既存の match を維持するため rustified enum のままにする。
- 生成された型はクレート内に閉じ込め、公開 API には次の独自型だけを出す。

### 公開型

- `AccelerationType` を追加する。

  ```rust
  pub enum AccelerationType {
      NotSupported,
      Hardware,
      Gpu,
      Software,
      Unknown(i32),
  }
  ```

- `VideoProfile` を追加し、既存の `H264Profile` / `HevcProfile` / `Av1Profile` を再利用する。

  ```rust
  pub enum VideoProfile {
      H264(H264Profile),
      Hevc(HevcProfile),
      Av1(Av1Profile),
      Unknown(i64),
  }
  ```

  H.264 の `AMF_VIDEO_ENCODER_PROFILE_UNKNOWN` (0) はプロファイル不明として `None` にする。それ以外の未知の値は `Unknown` に生値を保持する。HEVC と AV1 の値 0 は既知のプロファイルではないため `Unknown(0)` として保持する。

- `SurfaceFormat` を追加する。既存の `FrameFormat` に写像できる値は `Known` に、写像できない値は `Unknown` に生値を保持する。

  ```rust
  pub enum SurfaceFormat {
      Known(FrameFormat),
      Unknown(i32),
  }
  ```

  `FrameFormat` はエンコーダー入力の契約として使う既存の型であり、`SurfaceFormat` は caps の表示専用として分ける。`FrameFormat` に `Unknown` を追加すると、`EncoderConfig` に未知のフォーマットを渡せる型になってしまうため、caps 用の型を別に設ける。AMF の `GRAY8` や `R16G16` など `FrameFormat` にない値は `Unknown` に生値を保持する。

- `MemoryType` を追加する。Linux で実際に返る値だけを型にし、それ以外は `Unknown` に生値を保持する。

  ```rust
  pub enum MemoryType {
      Host,
      OpenCl,
      OpenGl,
      Xv,
      Vulkan,
      Unknown(i32),
  }
  ```

  `DX9` / `DX11` / `DX12` / `GRALLOC` / `COMPUTE_FOR_DX9` / `COMPUTE_FOR_DX11` はこのクレートでは使わないため `Unknown(raw)` にする。`AMF_MEMORY_UNKNOWN` (0) も `Unknown(0)` である。

- `IoCaps` を追加する。AMF が返した順序を維持する。

  ```rust
  pub struct IoCaps {
      pub min_width: i32,
      pub max_width: i32,
      pub min_height: i32,
      pub max_height: i32,
      pub vertical_alignment: i32,
      pub formats: Vec<SurfaceFormatCapability>,
      pub memory_types: Vec<MemoryTypeCapability>,
      pub interlaced: bool,
  }

  pub struct SurfaceFormatCapability {
      pub format: SurfaceFormat,
      pub native: bool,
  }

  pub struct MemoryTypeCapability {
      pub memory_type: MemoryType,
      pub native: bool,
  }
  ```

- `Property` と `Value` を追加する。`AMFCaps` の全プロパティを名前と値の組で保持する。

  ```rust
  pub struct Property {
      pub name: String,
      pub value: Value,
  }

  pub enum Value {
      Empty,
      Bool(bool),
      Int64(i64),
      Double(f64),
      Rect { left: i32, top: i32, right: i32, bottom: i32 },
      Size { width: i32, height: i32 },
      Point { x: i32, y: i32 },
      Rate { num: u32, den: u32 },
      Ratio { num: u32, den: u32 },
      Color { r: u8, g: u8, b: u8, a: u8 },
      String(String),
      WString(String),
      Interface,
      Float(f32),
      FloatSize { width: f32, height: f32 },
      FloatPoint2D { x: f32, y: f32 },
      FloatPoint3D { x: f32, y: f32, z: f32 },
      FloatVector4D { x: f32, y: f32, z: f32, w: f32 },
      Unknown { type_id: i32, raw: [u8; 16] },
  }
  ```

  `Interface` は AMF のインターフェース値の存在だけを表し、ポインタは公開しない。参照は変換後に `Release` する。未知の variant 型は `Unknown` に型 ID と 16 バイトの生値を保持する。

- `CodecInfo` / `DecodingInfo` / `EncodingInfo` を次の形にする。

  ```rust
  pub struct CodecInfo {
      pub codec: VideoCodecType,
      pub decoding: Option<DecodingInfo>,
      pub encoding: Option<EncodingInfo>,
  }

  pub struct DecodingInfo {
      pub acceleration: Option<AccelerationType>,
      pub input: Option<IoCaps>,
      pub output: Option<IoCaps>,
      pub properties: Vec<Property>,
  }

  pub struct EncodingInfo {
      pub acceleration: Option<AccelerationType>,
      pub max_profile: Option<VideoProfile>,
      pub max_level: Option<i64>,
      pub supports_frame_reordering: Option<bool>,
      pub input: Option<IoCaps>,
      pub output: Option<IoCaps>,
      pub properties: Vec<Property>,
  }
  ```

- `properties` は `AMFCaps` の全プロパティを `GetPropertyAt` の順序で保持する。`max_profile` / `max_level` / `supports_frame_reordering` に使うプロパティも除外せず、そのまま含める。
- `max_profile` / `max_level` / `supports_frame_reordering` は制御に使う値として構造体のフィールドに持つ。`properties` は値を確認するためのものであり、これを見て動作を切り替えることは想定しない。doc にこの役割分担を書く。
- `max_profile` は `MaxProfile` / `HevcMaxProfile` / `Av1MaxProfile`、`max_level` は `MaxLevel` / `HevcMaxLevel` / `Av1MaxLevel`、`supports_frame_reordering` は `BFrames` / `AV1BFrames` から取得する。プロパティが存在しない場合は `None` にする。
- `DecodingInfo` にはプロファイルとレベルのプロパティが存在しないため、専用のフィールドは設けない。`properties` にはデコーダーの全プロパティが入る。
- `profiles: EncodingProfiles`、`H264EncodingProfile` / `HevcEncodingProfile` / `Av1EncodingProfile`、`supported: bool`、`hardware_accelerated`、`supports_multi_pass` は削除する。

### `supported_codecs`

- 戻り値を `Result<Vec<CodecInfo>, Error>` に変更する。AMF ランタイムのロードまたは `Context` の作成に失敗した場合は `Err` を返す。個々のコーデックの `CreateComponent` または `Init` に失敗した場合は、その方向を `None` として返す。
- 返す順序は H.264 / HEVC / AV1 のままにする。
- 処理手順は次の通り。
  1. `Context` を 1 つ作る。
  2. コーデックごとに `CreateComponent` する。
  3. `GetCaps` を呼ぶ。成功した場合は入力 caps の最小幅・最小高さ、全プロパティ、入出力の `IoCaps` を取得する。
  4. エンコーダーは `NV12` と入力 caps の最小サイズで `Init` する。入力 caps を取得できない場合は 640x480 で `Init` する。デコーダーは `NV12`、0x0 で `Init` する。
  5. `Init` に成功したら `DecodingInfo` / `EncodingInfo` を作る。`GetCaps` が `Init` 前に失敗していた場合は、`Init` 後にもう一度 `GetCaps` を呼ぶ。
  6. `Terminate` してコンポーネントを解放する。
- `GetCaps` が `Init` 前に成功するかは AMF のバージョンに依存し得るため、`Init` 前の取得に失敗しても `Init` 後に再試行する手順にして、どちらの挙動でも動作させる。
- `AMFCaps` / `AMFIOCaps` の値は AMF が報告する能力であり、実際のエンコード・デコードの成功を保証しない。`supported_codecs` の doc に書く。

### 0040 との関係

- 0040 は 0022 に統合する。`supported_codecs` が全コーデック分の `CodecInfo` を返し、1 コーデックだけ必要な場合も結果を codec で絞れるため、`query_encoder_caps` / `query_decoder_caps` は追加しない。
- 0040 の設計に列挙されていた最大ビットレート、ROI、pre-analysis、最大テンポラルレイヤー数などは、0022 の `properties` に含まれる。typed accessor は追加しない。

### ドキュメントとテスト

- `EncodingInfo` / `DecodingInfo` / `IoCaps` / `Property` の各フィールドに、値の出所、単位、`None` の条件、順序を書く。
- `max_level` はコーデックごとのレベル enum の値であり、コーデック間で比較できないことを書く。
- 実機テストを追加する。`AMFCaps` から取得したプロパティ数、各プロパティの名前と値、`IoCaps` の幅・高さ、フォーマット数、メモリタイプ数、native フラグ、インターレース対応を `supported_codecs` の出力と比較する。
- `properties` に `MaxProfile` / `MaxLevel` / `BFrames` など、構造体のフィールドに写像したプロパティも含まれることを確認する。
- `EncodingInfo::max_profile` / `max_level` / `supports_frame_reordering` が対応するプロパティの値と一致することを確認する。
- `supported_codecs` が `Ok` を返すこと、要素の順序、`Some(encoding)` の `max_profile` の variant が `codec` と一致することを確認する。
- `HEVC` の `BFrames` が存在しない場合に `None` になることを実機で確認する。
- `examples/amf-check`、`README.md`、`skills/shiguredo-amf/SKILL.md` を新しい API に合わせる。
- `CHANGES.md` の `## develop` に `[CHANGE]` を追記する。

## 完了条件

- `supported_codecs` が `Result` を返し、AMF ランタイム / `Context` の失敗と、個々のコーデック非対応を区別できる。
- `CodecInfo` が解像度、フォーマット、メモリタイプ、インターレース対応、全プロパティ、アクセラレーション種別を返す。
- `properties` に `MaxProfile` / `MaxLevel` / `BFrames` など、構造体のフィールドに写像したプロパティも含まれる。
- `properties` は値を確認するためのものであり、制御には構造体のフィールドを使うことが doc に書かれている。
- `AMFCaps` のプロパティ数と `properties.len()`、`AMFIOCaps` のフォーマット数と `formats.len()`、メモリタイプ数と `memory_types.len()` が実機で一致する。
- `VideoProfile::Unknown`、`AccelerationType::Unknown`、`SurfaceFormat::Unknown`、`MemoryType::Unknown`、`Value::Unknown` が生値を保持する。
- `hardware_accelerated` が削除され、`AccelerationType` で AMF の値を保持する。
- `GetCaps` が `Init` 前後どちらでも動く手順になっている。
- 実機テストが追加され、`cargo test --workspace` が通る。
- `examples/amf-check`、`README.md`、`skills/shiguredo-amf/SKILL.md` が新しい API に追従している。
- `CHANGES.md` の `## develop` に変更が追記されている。
