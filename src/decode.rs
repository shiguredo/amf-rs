//! AMF ハードウェアデコーダー
//!
//! AMD GPU を使ったハードウェアビデオデコードを提供する。
//! H.264/AVC、H.265/HEVC、AV1 コーデックに対応する。

use std::collections::VecDeque;
use std::ptr;

use crate::AmfLibrary;
use crate::error::{Error, positive_i32_to_usize, require_vtbl_fn};
use crate::sys::{
    self, AMF_MEMORY_TYPE, AMF_PLANE_TYPE, AMF_RESULT, AMF_SURFACE_FORMAT, AMFBuffer, AMFComponent,
    AMFContext, AMFData, AMFSurface,
};

// ---------------------------------------------------------------------------
// 公開型
// ---------------------------------------------------------------------------

/// デコーダーのコーデック種別
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecoderCodec {
    H264,
    Hevc,
    Av1,
}

/// デコーダー設定
#[derive(Debug, Clone)]
pub struct DecoderConfig {
    pub codec: DecoderCodec,
}

/// デコード済みフレーム (NV12 形式)
pub struct DecodedFrame {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl DecodedFrame {
    /// NV12 フレームデータへの参照
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// フレームデータの所有権を取得する
    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    /// フレーム幅
    pub fn width(&self) -> usize {
        self.width
    }

    /// フレーム高さ
    pub fn height(&self) -> usize {
        self.height
    }
}

// ---------------------------------------------------------------------------
// デコーダー実装
// ---------------------------------------------------------------------------

/// AMF ハードウェアデコーダー
pub struct Decoder {
    // ライブラリハンドルを保持して Drop 順序で dlclose が最後に呼ばれることを保証する
    _lib: AmfLibrary,
    context: *mut AMFContext,
    component: *mut AMFComponent,
    decoded_frames: VecDeque<DecodedFrame>,
}

// 安全性:
// AMF のコンテキストとコンポーネントはスレッドセーフではない (同時アクセス不可) が、
// 所有権の移動は許可されている。AMF API Reference では単一スレッドからの逐次アクセスを
// 要求しており、スレッド親和性 (特定スレッドへの束縛) は要求していない。
// Vulkan バックエンドの VkDevice/VkQueue も Vulkan 仕様上スレッド間移動が可能で、
// 同時アクセスのみ外部同期が必要とされる (Vulkan Spec §3.6)。
//
// したがって Send (所有権移動) は安全だが、Sync (共有参照による同時アクセス) は安全ではない。
unsafe impl Send for Decoder {}

impl Decoder {
    /// デコーダーを作成する
    pub fn new(config: DecoderConfig) -> Result<Self, Error> {
        let lib = AmfLibrary::load()?;
        let context = lib.create_context()?;

        AmfLibrary::init_vulkan(context)?;

        let component_id = match config.codec {
            DecoderCodec::H264 => sys::str::AMFVideoDecoderUVD_H264_AVC,
            DecoderCodec::Hevc => sys::str::AMFVideoDecoderHW_H265_HEVC,
            DecoderCodec::Av1 => sys::str::AMFVideoDecoderHW_AV1,
        };
        let component_id_w = sys::to_wstring(component_id);

        let mut component: *mut AMFComponent = ptr::null_mut();
        let result = unsafe {
            let vtbl = &*(*lib.factory()).pVtbl;
            require_vtbl_fn(vtbl.CreateComponent, "CreateComponent")?(
                lib.factory(),
                context,
                component_id_w.as_ptr(),
                &mut component,
            )
        };
        Error::check(result, "AMFFactory::CreateComponent")?;

        if component.is_null() {
            return Err(Error::new_custom(
                "Decoder::new",
                "CreateComponent returned null",
            ));
        }

        // デコーダーを初期化する (解像度は 0,0 でストリームから自動検出)
        let result = unsafe {
            let vtbl = &*(*component).pVtbl;
            require_vtbl_fn(vtbl.Init, "Init")?(
                component,
                AMF_SURFACE_FORMAT::AMF_SURFACE_NV12,
                0,
                0,
            )
        };
        Error::check(result, "AMFComponent::Init")?;

        Ok(Self {
            _lib: lib,
            context,
            component,
            decoded_frames: VecDeque::new(),
        })
    }

    /// ビットストリームをデコードする
    ///
    /// エンコーダーが出力した 1 フレーム分のデータを渡すこと。
    /// 複数フレームを連結したビットストリームの一括送信は
    /// AMF デコーダーの制約により失敗する場合がある。
    pub fn decode(&mut self, data: &[u8]) -> Result<(), Error> {
        if data.is_empty() {
            return Ok(());
        }

        log::debug!("Decoder::decode: data size={}", data.len());

        // 入力バッファを確保する
        let mut buffer: *mut AMFBuffer = ptr::null_mut();
        let result = unsafe {
            let vtbl = &*(*self.context).pVtbl;
            require_vtbl_fn(vtbl.AllocBuffer, "AllocBuffer")?(
                self.context,
                AMF_MEMORY_TYPE::AMF_MEMORY_HOST,
                data.len(),
                &mut buffer,
            )
        };
        Error::check(result, "AMFContext::AllocBuffer")?;

        if buffer.is_null() {
            return Err(Error::new_custom(
                "Decoder::decode",
                "AllocBuffer returned null",
            ));
        }

        // データをコピーする
        let buf_native = unsafe {
            let vtbl = &*(*buffer).pVtbl;
            require_vtbl_fn(vtbl.GetNative, "GetNative")?(buffer) as *mut u8
        };
        if buf_native.is_null() {
            return Err(Error::new_custom(
                "Decoder::decode",
                "buffer native pointer is null",
            ));
        }
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), buf_native, data.len());
        }

        // SubmitInput (デバイスビジーの場合はリトライする)
        // AMF の SubmitInput は成功時にバッファの所有権を取得するため、
        // 成功パスでは Release を呼ばない。
        let max_retries = 100;
        let mut submitted = false;
        for retry in 0..max_retries {
            let result = unsafe {
                let vtbl = &*(*self.component).pVtbl;
                require_vtbl_fn(vtbl.SubmitInput, "SubmitInput")?(
                    self.component,
                    buffer as *mut AMFData,
                )
            };

            if retry > 0 && retry % 10 == 0 {
                log::debug!("Decoder::SubmitInput: retry={retry}, result={result:?}");
            }
            if result == AMF_RESULT::AMF_OK || result == AMF_RESULT::AMF_NEED_MORE_INPUT {
                log::debug!("Decoder::SubmitInput: accepted, result={result:?}");
                submitted = true;
                break;
            }
            if result == AMF_RESULT::AMF_INPUT_FULL
                || result == AMF_RESULT::AMF_DECODER_NO_FREE_SURFACES
                || result == AMF_RESULT::AMF_REPEAT
            {
                // 出力をポーリングしてからリトライする
                self.poll_output()?;
                std::thread::sleep(std::time::Duration::from_millis(1));
                continue;
            }
            if result == AMF_RESULT::AMF_EOF {
                submitted = true;
                break;
            }
            // バッファを解放してエラーを返す
            unsafe {
                let vtbl = &*(*buffer).pVtbl;
                if let Some(release) = vtbl.Release {
                    release(buffer);
                }
            }
            return Error::check(result, "AMFComponent::SubmitInput");
        }
        if !submitted {
            unsafe {
                let vtbl = &*(*buffer).pVtbl;
                if let Some(release) = vtbl.Release {
                    release(buffer);
                }
            }
            return Err(Error::new_custom(
                "Decoder::decode",
                "SubmitInput retry limit exceeded",
            ));
        }

        // 出力をポーリングする
        self.poll_output()?;

        Ok(())
    }

    /// デコーダーをフラッシュして残りのフレームを取得する
    pub fn finish(&mut self) -> Result<(), Error> {
        log::debug!("Decoder::finish: calling Drain");
        // Drain を呼び出す
        let result = unsafe {
            let vtbl = &*(*self.component).pVtbl;
            require_vtbl_fn(vtbl.Drain, "Drain")?(self.component)
        };
        log::debug!("Decoder::finish: Drain result={result:?}");
        if result != AMF_RESULT::AMF_OK && result != AMF_RESULT::AMF_INPUT_FULL {
            Error::check(result, "AMFComponent::Drain")?;
        }

        // 残りの出力を取得する
        // AMF_REPEAT はデコード中で出力が未準備の意味なのでリトライする。
        // Vulkan バックエンドでは非同期処理のため完了まで待機が必要。
        let max_repeat = 50;
        let mut flush_count = 0;
        let mut repeat_count = 0;
        loop {
            let mut data: *mut AMFData = ptr::null_mut();
            let result = unsafe {
                let vtbl = &*(*self.component).pVtbl;
                require_vtbl_fn(vtbl.QueryOutput, "QueryOutput")?(self.component, &mut data)
            };
            if result == AMF_RESULT::AMF_EOF {
                break;
            }
            if result == AMF_RESULT::AMF_REPEAT {
                if !data.is_null() {
                    // AMF_REPEAT でも data が返っている場合はフレームを抽出する
                    repeat_count = 0;
                    self.extract_frame(data as *mut AMFSurface)?;
                    flush_count += 1;
                    continue;
                }
                repeat_count += 1;
                if repeat_count > max_repeat {
                    log::debug!(
                        "Decoder::finish: AMF_REPEAT retry limit, flush_count={flush_count}"
                    );
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            if result != AMF_RESULT::AMF_OK || data.is_null() {
                break;
            }

            repeat_count = 0;
            self.extract_frame(data as *mut AMFSurface)?;
            flush_count += 1;
        }
        log::debug!("Decoder::finish: done, flushed {flush_count} frames");

        Ok(())
    }

    /// デコード済みフレームを取り出す
    pub fn next_frame(&mut self) -> Option<DecodedFrame> {
        self.decoded_frames.pop_front()
    }

    /// 出力をポーリングする
    fn poll_output(&mut self) -> Result<(), Error> {
        loop {
            let mut data: *mut AMFData = ptr::null_mut();
            let result = unsafe {
                let vtbl = &*(*self.component).pVtbl;
                require_vtbl_fn(vtbl.QueryOutput, "QueryOutput")?(self.component, &mut data)
            };

            if result == AMF_RESULT::AMF_EOF {
                break;
            }
            if result == AMF_RESULT::AMF_REPEAT {
                if !data.is_null() {
                    // AMF_REPEAT でも data が返っている場合はフレームを抽出する
                    self.extract_frame(data as *mut AMFSurface)?;
                    continue;
                }
                break;
            }
            if result != AMF_RESULT::AMF_OK || data.is_null() {
                break;
            }

            self.extract_frame(data as *mut AMFSurface)?;
        }
        Ok(())
    }

    /// AMFSurface から NV12 フレームデータを抽出する
    ///
    /// null ポインタやサイズ不整合によるメモリアクセス違反を防ぐため、
    /// 各段階で検証を行う。
    fn extract_frame(&mut self, surface: *mut AMFSurface) -> Result<(), Error> {
        if surface.is_null() {
            return Err(Error::new_custom("extract_frame", "surface is null"));
        }

        /// Surface のクリーンアップヘルパー
        ///
        /// vtable の Release が欠けている場合はリークさせて panic を防ぐ
        unsafe fn release_surface(surface: *mut AMFSurface) {
            unsafe {
                let vtbl = &*(*surface).pVtbl;
                if let Some(release) = vtbl.Release {
                    release(surface);
                }
            }
        }

        // ホストメモリに変換する
        let result = unsafe {
            let vtbl = &*(*surface).pVtbl;
            require_vtbl_fn(vtbl.Convert, "Convert")?(surface, AMF_MEMORY_TYPE::AMF_MEMORY_HOST)
        };
        if result != AMF_RESULT::AMF_OK {
            unsafe { release_surface(surface) };
            return Error::check(result, "AMFSurface::Convert");
        }

        // Y プレーンを取得する
        let y_plane = unsafe {
            let vtbl = &*(*surface).pVtbl;
            require_vtbl_fn(vtbl.GetPlane, "GetPlane")?(surface, AMF_PLANE_TYPE::AMF_PLANE_Y)
        };
        if y_plane.is_null() {
            unsafe { release_surface(surface) };
            return Err(Error::new_custom("extract_frame", "failed to get Y plane"));
        }

        let width_raw = unsafe {
            let vtbl = &*(*y_plane).pVtbl;
            require_vtbl_fn(vtbl.GetWidth, "GetWidth")?(y_plane)
        };
        let height_raw = unsafe {
            let vtbl = &*(*y_plane).pVtbl;
            require_vtbl_fn(vtbl.GetHeight, "GetHeight")?(y_plane)
        };
        let width = positive_i32_to_usize(width_raw, "extract_frame", "width")?;
        let height = positive_i32_to_usize(height_raw, "extract_frame", "height")?;
        let y_hpitch_raw = unsafe {
            let vtbl = &*(*y_plane).pVtbl;
            require_vtbl_fn(vtbl.GetHPitch, "GetHPitch")?(y_plane)
        };
        let y_hpitch = positive_i32_to_usize(y_hpitch_raw, "extract_frame", "y_hpitch")?;
        if y_hpitch < width {
            unsafe { release_surface(surface) };
            return Err(Error::new_custom(
                "extract_frame",
                &format!("Y hpitch ({y_hpitch}) < width ({width})"),
            ));
        }
        let y_native = unsafe {
            let vtbl = &*(*y_plane).pVtbl;
            require_vtbl_fn(vtbl.GetNative, "GetNative")?(y_plane) as *const u8
        };
        if y_native.is_null() {
            unsafe { release_surface(surface) };
            return Err(Error::new_custom(
                "extract_frame",
                "Y plane native pointer is null",
            ));
        }

        let y_size = width
            .checked_mul(height)
            .ok_or_else(|| Error::new_custom("extract_frame", "Y plane size overflow"))?;
        let nv12_size = y_size
            .checked_mul(3)
            .and_then(|v| v.checked_div(2))
            .ok_or_else(|| Error::new_custom("extract_frame", "NV12 size overflow"))?;
        let mut frame_data = vec![0u8; nv12_size];

        // Y プレーンをコピーする
        for row in 0..height {
            unsafe {
                ptr::copy_nonoverlapping(
                    y_native.add(row * y_hpitch),
                    frame_data.as_mut_ptr().add(row * width),
                    width,
                );
            }
        }

        // UV プレーンを取得する
        let uv_plane = unsafe {
            let vtbl = &*(*surface).pVtbl;
            require_vtbl_fn(vtbl.GetPlane, "GetPlane")?(surface, AMF_PLANE_TYPE::AMF_PLANE_UV)
        };
        if !uv_plane.is_null() {
            let uv_hpitch_raw = unsafe {
                let vtbl = &*(*uv_plane).pVtbl;
                require_vtbl_fn(vtbl.GetHPitch, "GetHPitch")?(uv_plane)
            };
            let uv_hpitch = uv_hpitch_raw.max(0) as usize;
            let uv_height_raw = unsafe {
                let vtbl = &*(*uv_plane).pVtbl;
                require_vtbl_fn(vtbl.GetHeight, "GetHeight")?(uv_plane)
            };
            let uv_height = uv_height_raw.max(0) as usize;
            let uv_native = unsafe {
                let vtbl = &*(*uv_plane).pVtbl;
                require_vtbl_fn(vtbl.GetNative, "GetNative")?(uv_plane) as *const u8
            };
            if !uv_native.is_null() && uv_hpitch >= width && uv_height > 0 {
                // UV プレーンの高さが NV12 バッファの残り領域を超えないか検証する
                let max_uv_rows = (nv12_size - y_size) / width;
                let copy_rows = uv_height.min(max_uv_rows);
                for row in 0..copy_rows {
                    unsafe {
                        ptr::copy_nonoverlapping(
                            uv_native.add(row * uv_hpitch),
                            frame_data.as_mut_ptr().add(y_size + row * width),
                            width,
                        );
                    }
                }
            }
        }

        // Surface を解放する
        unsafe { release_surface(surface) };

        self.decoded_frames.push_back(DecodedFrame {
            width,
            height,
            data: frame_data,
        });

        Ok(())
    }
}

// 安全性: new() が成功した場合のみ Self が構築されるため、
// component と context は常に有効なポインタであることが保証される。
impl Drop for Decoder {
    fn drop(&mut self) {
        // Drop 内の panic は二重 panic で abort になるため、
        // vtable の関数ポインタが欠けている場合は握りつぶす
        unsafe {
            let vtbl = &*(*self.component).pVtbl;
            if let Some(terminate) = vtbl.Terminate {
                let _ = terminate(self.component);
            }
            if let Some(release) = vtbl.Release {
                release(self.component);
            }

            let vtbl = &*(*self.context).pVtbl;
            if let Some(terminate) = vtbl.Terminate {
                let _ = terminate(self.context);
            }
            if let Some(release) = vtbl.Release {
                release(self.context);
            }
        }
    }
}
