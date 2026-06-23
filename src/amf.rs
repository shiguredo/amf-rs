//! AMF オブジェクトの RAII ラッパー
//!
//! `*mut AMFSurface` のような生ポインタをラップし、
//! Clone/Drop で `Acquire`/`Release` による参照カウント管理を自動化する。
//! vtable の関数ポインタは初期化時にすべて取り出して構造体メンバーに保持するため、
//! メソッド呼び出しのたびに vtable 検証は行われない。

use std::ptr;

use crate::error::{Error, require_vtbl_fn};
use crate::sys;

// ---------------------------------------------------------------------------
// PropertyStorage
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct PropertyStorageFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFPropertyStorage) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFPropertyStorage) -> sys::amf_long,
    set_property: unsafe extern "C" fn(
        *mut sys::AMFPropertyStorage,
        *const sys::wchar_t,
        sys::AMFVariantStruct,
    ) -> sys::AMF_RESULT,
}

/// `AMFPropertyStorage` の RAII ラッパー
///
/// `Surface` や `Component` から `property_storage()` で取得する。
/// 取得時に `Acquire` が呼ばれ、`Drop` 時に `Release` が呼ばれる。
pub struct PropertyStorage {
    ptr: *mut sys::AMFPropertyStorage,
    f: PropertyStorageFuncs,
}

unsafe impl Send for PropertyStorage {}

impl PropertyStorage {
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFPropertyStorage,
        acquire: bool,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = PropertyStorageFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFPropertyStorage::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFPropertyStorage::Release")?,
            set_property: require_vtbl_fn(vtbl.SetProperty, "AMFPropertyStorage::SetProperty")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        Ok(Self { ptr, f })
    }

    /// 生ポインタから PropertyStorage を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は有効な AMFPropertyStorage を指すこと。
    pub unsafe fn from_raw(ptr: *mut sys::AMFPropertyStorage) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false) }
    }

    /// 生ポインタから Acquire 付きで PropertyStorage を作成する
    ///
    /// # Safety
    /// `ptr` は有効な AMFPropertyStorage を指すこと。
    pub unsafe fn from_raw_acquired(ptr: *mut sys::AMFPropertyStorage) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true) }
    }

    /// 生ポインタを返す
    pub fn as_ptr(&self) -> *mut sys::AMFPropertyStorage {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)
    pub fn into_raw(self) -> *mut sys::AMFPropertyStorage {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }

    /// プロパティを設定する
    ///
    /// # Safety
    /// `name` は有効なヌル終端ワイド文字列を指すこと。
    pub unsafe fn set_property(
        &self,
        name: *const sys::wchar_t,
        value: sys::AMFVariantStruct,
    ) -> sys::AMF_RESULT {
        unsafe { (self.f.set_property)(self.ptr, name, value) }
    }

    /// Int64 プロパティを設定する
    pub fn set_property_int64(&self, name: &str, value: sys::amf_int64) -> Result<(), Error> {
        let name_w = sys::to_wstring(name);
        let var = sys::AMFVariantStruct::from_int64(value);
        let result = unsafe { self.set_property(name_w.as_ptr(), var) };
        Error::check(result, format!("SetProperty({name})"))
    }

    /// Size プロパティを設定する
    pub fn set_property_size(
        &self,
        name: &str,
        width: sys::amf_int32,
        height: sys::amf_int32,
    ) -> Result<(), Error> {
        let name_w = sys::to_wstring(name);
        let var = sys::AMFVariantStruct::from_size(width, height);
        let result = unsafe { self.set_property(name_w.as_ptr(), var) };
        Error::check(result, format!("SetProperty({name})"))
    }

    /// Rate プロパティを設定する
    pub fn set_property_rate(&self, name: &str, num: u32, den: u32) -> Result<(), Error> {
        let name_w = sys::to_wstring(name);
        let var = sys::AMFVariantStruct::from_rate(num, den);
        let result = unsafe { self.set_property(name_w.as_ptr(), var) };
        Error::check(result, format!("SetProperty({name})"))
    }
}

impl Clone for PropertyStorage {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        Self {
            ptr: self.ptr,
            f: self.f,
        }
    }
}

impl Drop for PropertyStorage {
    fn drop(&mut self) {
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// Surface
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct SurfaceFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFSurface) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFSurface) -> sys::amf_long,
    set_pts: unsafe extern "C" fn(*mut sys::AMFSurface, sys::amf_pts),
    set_duration: unsafe extern "C" fn(*mut sys::AMFSurface, sys::amf_pts),
    get_plane:
        unsafe extern "C" fn(*mut sys::AMFSurface, sys::AMF_PLANE_TYPE) -> *mut sys::AMFPlane,
    get_plane_at: unsafe extern "C" fn(*mut sys::AMFSurface, sys::amf_size) -> *mut sys::AMFPlane,
    convert: unsafe extern "C" fn(*mut sys::AMFSurface, sys::AMF_MEMORY_TYPE) -> sys::AMF_RESULT,
}

/// `AMFSurface` の RAII ラッパー
#[derive(Debug)]
pub struct Surface {
    ptr: *mut sys::AMFSurface,
    f: SurfaceFuncs,
}

unsafe impl Send for Surface {}

impl Surface {
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFSurface,
        acquire: bool,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = SurfaceFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFSurface::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFSurface::Release")?,
            set_pts: require_vtbl_fn(vtbl.SetPts, "AMFSurface::SetPts")?,
            set_duration: require_vtbl_fn(vtbl.SetDuration, "AMFSurface::SetDuration")?,
            get_plane: require_vtbl_fn(vtbl.GetPlane, "AMFSurface::GetPlane")?,
            get_plane_at: require_vtbl_fn(vtbl.GetPlaneAt, "AMFSurface::GetPlaneAt")?,
            convert: require_vtbl_fn(vtbl.Convert, "AMFSurface::Convert")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        Ok(Self { ptr, f })
    }

    /// 生ポインタから Surface を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は有効な `AMFSurface` を指すこと。
    pub unsafe fn from_raw(ptr: *mut sys::AMFSurface) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false) }
    }

    /// 生ポインタから Acquire 付きで Surface を作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFSurface` を指すこと。
    pub unsafe fn from_raw_acquired(ptr: *mut sys::AMFSurface) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true) }
    }

    /// 生ポインタを返す
    pub fn as_ptr(&self) -> *mut sys::AMFSurface {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)
    pub fn into_raw(self) -> *mut sys::AMFSurface {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }

    /// PTS を設定する
    pub fn set_pts(&self, pts: sys::amf_pts) {
        unsafe { (self.f.set_pts)(self.ptr, pts) }
    }

    /// Duration を設定する
    pub fn set_duration(&self, duration: sys::amf_pts) {
        unsafe { (self.f.set_duration)(self.ptr, duration) }
    }

    /// 指定プレーンを取得する
    pub fn get_plane(&self, plane_type: sys::AMF_PLANE_TYPE) -> Result<Plane, Error> {
        let plane = unsafe { (self.f.get_plane)(self.ptr, plane_type) };
        if plane.is_null() {
            return Err(Error::new_custom("Surface::get_plane", "plane is null"));
        }
        unsafe { Plane::from_raw_acquired(plane) }
    }

    /// インデックスでプレーンを取得する
    pub fn get_plane_at(&self, index: sys::amf_size) -> Result<Plane, Error> {
        let plane = unsafe { (self.f.get_plane_at)(self.ptr, index) };
        if plane.is_null() {
            return Err(Error::new_custom("Surface::get_plane_at", "plane is null"));
        }
        unsafe { Plane::from_raw_acquired(plane) }
    }

    /// GPU → Host メモリ変換
    pub fn convert(&self, type_: sys::AMF_MEMORY_TYPE) -> sys::AMF_RESULT {
        unsafe { (self.f.convert)(self.ptr, type_) }
    }

    /// AMFPropertyStorage としてアクセスする
    ///
    /// 戻り値の `PropertyStorage` が `Drop` されるまで参照カウントが増える。
    pub fn property_storage(&self) -> Result<PropertyStorage, Error> {
        unsafe { PropertyStorage::from_raw_acquired(self.ptr as *mut sys::AMFPropertyStorage) }
    }
}

impl Clone for Surface {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        Self {
            ptr: self.ptr,
            f: self.f,
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// Buffer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct BufferFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_long,
    get_native: unsafe extern "C" fn(*mut sys::AMFBuffer) -> *mut std::ffi::c_void,
    get_size: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_size,
    get_pts: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_pts,
    get_duration: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_pts,
    get_property: unsafe extern "C" fn(
        *mut sys::AMFBuffer,
        *const sys::wchar_t,
        *mut sys::AMFVariantStruct,
    ) -> sys::AMF_RESULT,
}

/// `AMFBuffer` の RAII ラッパー
#[derive(Debug)]
pub struct Buffer {
    ptr: *mut sys::AMFBuffer,
    f: BufferFuncs,
}

unsafe impl Send for Buffer {}

impl Buffer {
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFBuffer,
        acquire: bool,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = BufferFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFBuffer::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFBuffer::Release")?,
            get_native: require_vtbl_fn(vtbl.GetNative, "AMFBuffer::GetNative")?,
            get_size: require_vtbl_fn(vtbl.GetSize, "AMFBuffer::GetSize")?,
            get_pts: require_vtbl_fn(vtbl.GetPts, "AMFBuffer::GetPts")?,
            get_duration: require_vtbl_fn(vtbl.GetDuration, "AMFBuffer::GetDuration")?,
            get_property: require_vtbl_fn(vtbl.GetProperty, "AMFBuffer::GetProperty")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        Ok(Self { ptr, f })
    }

    /// 生ポインタから Buffer を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は有効な `AMFBuffer` を指すこと。
    pub unsafe fn from_raw(ptr: *mut sys::AMFBuffer) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false) }
    }

    /// 生ポインタから Acquire 付きで Buffer を作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFBuffer` を指すこと。
    pub unsafe fn from_raw_acquired(ptr: *mut sys::AMFBuffer) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true) }
    }

    /// 生ポインタを返す
    pub fn as_ptr(&self) -> *mut sys::AMFBuffer {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)
    pub fn into_raw(self) -> *mut sys::AMFBuffer {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }

    /// バッファのネイティブポインタを取得する
    pub fn get_native(&self) -> *mut std::ffi::c_void {
        unsafe { (self.f.get_native)(self.ptr) }
    }

    /// バッファサイズを取得する
    pub fn get_size(&self) -> sys::amf_size {
        unsafe { (self.f.get_size)(self.ptr) }
    }

    /// PTS を取得する
    pub fn get_pts(&self) -> sys::amf_pts {
        unsafe { (self.f.get_pts)(self.ptr) }
    }

    /// Duration を取得する
    pub fn get_duration(&self) -> sys::amf_pts {
        unsafe { (self.f.get_duration)(self.ptr) }
    }

    /// プロパティを取得する
    ///
    /// # Safety
    /// `name` は有効なヌル終端ワイド文字列を指すこと。
    /// `p_value` は有効な AMFVariantStruct を指すこと。
    pub unsafe fn get_property(
        &self,
        name: *const sys::wchar_t,
        p_value: *mut sys::AMFVariantStruct,
    ) -> sys::AMF_RESULT {
        unsafe { (self.f.get_property)(self.ptr, name, p_value) }
    }
}

impl Clone for Buffer {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        Self {
            ptr: self.ptr,
            f: self.f,
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// Plane
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct PlaneFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_long,
    get_native: unsafe extern "C" fn(*mut sys::AMFPlane) -> *mut std::ffi::c_void,
    get_hpitch: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_int32,
    get_vpitch: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_int32,
    get_width: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_int32,
    get_height: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_int32,
}

/// `AMFPlane` の RAII ラッパー
///
/// `Surface` から `get_plane()` / `get_plane_at()` で取得する。
/// 取得時に `Acquire` が呼ばれ、`Drop` 時に `Release` が呼ばれる。
pub struct Plane {
    ptr: *mut sys::AMFPlane,
    f: PlaneFuncs,
}

unsafe impl Send for Plane {}

impl Plane {
    unsafe fn from_raw_with_acquire(ptr: *mut sys::AMFPlane, acquire: bool) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = PlaneFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFPlane::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFPlane::Release")?,
            get_native: require_vtbl_fn(vtbl.GetNative, "AMFPlane::GetNative")?,
            get_hpitch: require_vtbl_fn(vtbl.GetHPitch, "AMFPlane::GetHPitch")?,
            get_vpitch: require_vtbl_fn(vtbl.GetVPitch, "AMFPlane::GetVPitch")?,
            get_width: require_vtbl_fn(vtbl.GetWidth, "AMFPlane::GetWidth")?,
            get_height: require_vtbl_fn(vtbl.GetHeight, "AMFPlane::GetHeight")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        Ok(Self { ptr, f })
    }

    /// 生ポインタから Plane を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は有効な `AMFPlane` を指すこと。
    pub unsafe fn from_raw(ptr: *mut sys::AMFPlane) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false) }
    }

    /// 生ポインタから Acquire 付きで Plane を作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFPlane` を指すこと。
    pub unsafe fn from_raw_acquired(ptr: *mut sys::AMFPlane) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true) }
    }

    /// 生ポインタを返す
    pub fn as_ptr(&self) -> *mut sys::AMFPlane {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)
    pub fn into_raw(self) -> *mut sys::AMFPlane {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }

    /// プレーンのネイティブポインタを取得する
    pub fn get_native(&self) -> *mut std::ffi::c_void {
        unsafe { (self.f.get_native)(self.ptr) }
    }

    /// 水平ピッチを取得する
    pub fn get_hpitch(&self) -> sys::amf_int32 {
        unsafe { (self.f.get_hpitch)(self.ptr) }
    }

    /// 垂直ピッチを取得する
    pub fn get_vpitch(&self) -> sys::amf_int32 {
        unsafe { (self.f.get_vpitch)(self.ptr) }
    }

    /// 幅を取得する
    pub fn get_width(&self) -> sys::amf_int32 {
        unsafe { (self.f.get_width)(self.ptr) }
    }

    /// 高さを取得する
    pub fn get_height(&self) -> sys::amf_int32 {
        unsafe { (self.f.get_height)(self.ptr) }
    }
}

impl Clone for Plane {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        Self {
            ptr: self.ptr,
            f: self.f,
        }
    }
}

impl Drop for Plane {
    fn drop(&mut self) {
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct ContextFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFContext) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFContext) -> sys::amf_long,
    terminate: unsafe extern "C" fn(*mut sys::AMFContext) -> sys::AMF_RESULT,
    alloc_surface: unsafe extern "C" fn(
        *mut sys::AMFContext,
        sys::AMF_MEMORY_TYPE,
        sys::AMF_SURFACE_FORMAT,
        sys::amf_int32,
        sys::amf_int32,
        *mut *mut sys::AMFSurface,
    ) -> sys::AMF_RESULT,
    alloc_buffer: unsafe extern "C" fn(
        *mut sys::AMFContext,
        sys::AMF_MEMORY_TYPE,
        sys::amf_size,
        *mut *mut sys::AMFBuffer,
    ) -> sys::AMF_RESULT,
    query_interface: unsafe extern "C" fn(
        *mut sys::AMFContext,
        *const sys::AMFGuid,
        *mut *mut std::ffi::c_void,
    ) -> sys::AMF_RESULT,
}

/// `AMFContext` の RAII ラッパー
pub struct Context {
    ptr: *mut sys::AMFContext,
    f: ContextFuncs,
}

unsafe impl Send for Context {}

impl Context {
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFContext,
        acquire: bool,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = ContextFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFContext::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFContext::Release")?,
            terminate: require_vtbl_fn(vtbl.Terminate, "AMFContext::Terminate")?,
            alloc_surface: require_vtbl_fn(vtbl.AllocSurface, "AMFContext::AllocSurface")?,
            alloc_buffer: require_vtbl_fn(vtbl.AllocBuffer, "AMFContext::AllocBuffer")?,
            query_interface: require_vtbl_fn(vtbl.QueryInterface, "AMFContext::QueryInterface")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        Ok(Self { ptr, f })
    }

    /// 生ポインタから Context を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は有効な `AMFContext` を指すこと。
    pub unsafe fn from_raw(ptr: *mut sys::AMFContext) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false) }
    }

    /// 生ポインタから Acquire 付きで Context を作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFContext` を指すこと。
    pub unsafe fn from_raw_acquired(ptr: *mut sys::AMFContext) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true) }
    }

    /// 生ポインタを返す
    pub fn as_ptr(&self) -> *mut sys::AMFContext {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)
    pub fn into_raw(self) -> *mut sys::AMFContext {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }

    /// コンテキストを終了する
    pub fn terminate(&self) -> sys::AMF_RESULT {
        unsafe { (self.f.terminate)(self.ptr) }
    }

    /// Surface を確保する
    pub fn alloc_surface(
        &self,
        type_: sys::AMF_MEMORY_TYPE,
        format: sys::AMF_SURFACE_FORMAT,
        width: sys::amf_int32,
        height: sys::amf_int32,
    ) -> Result<Surface, Error> {
        let mut surface: *mut sys::AMFSurface = ptr::null_mut();
        let result =
            unsafe { (self.f.alloc_surface)(self.ptr, type_, format, width, height, &mut surface) };
        Error::check(result, "AMFContext::AllocSurface")?;

        if surface.is_null() {
            return Err(Error::new_custom(
                "Context::alloc_surface",
                "AllocSurface returned null",
            ));
        }

        unsafe { Surface::from_raw(surface) }
    }

    /// Buffer を確保する
    pub fn alloc_buffer(
        &self,
        type_: sys::AMF_MEMORY_TYPE,
        size: sys::amf_size,
    ) -> Result<Buffer, Error> {
        let mut buffer: *mut sys::AMFBuffer = ptr::null_mut();
        let result = unsafe { (self.f.alloc_buffer)(self.ptr, type_, size, &mut buffer) };
        Error::check(result, "AMFContext::AllocBuffer")?;

        if buffer.is_null() {
            return Err(Error::new_custom(
                "Context::alloc_buffer",
                "AllocBuffer returned null",
            ));
        }

        unsafe { Buffer::from_raw(buffer) }
    }

    /// QueryInterface で AMFContext1 を取得し Vulkan デバイスを初期化する
    ///
    /// `device` が `null` の場合はデフォルトデバイスが使用される。
    ///
    /// # Safety
    /// `device` は有効な Vulkan デバイスを指すこと。
    pub unsafe fn init_vulkan(&self, device: *mut std::ffi::c_void) -> Result<(), Error> {
        let mut context1_ptr: *mut std::ffi::c_void = ptr::null_mut();
        let result = unsafe {
            (self.f.query_interface)(self.ptr, &sys::IID_AMF_CONTEXT1, &mut context1_ptr)
        };
        Error::check(result, "AMFContext::QueryInterface(AMFContext1)")?;

        if context1_ptr.is_null() {
            return Err(Error::new_custom(
                "Context::init_vulkan",
                "QueryInterface returned null for AMFContext1",
            ));
        }

        let context1 = context1_ptr as *mut sys::AMFContext1;

        let result = unsafe {
            let vtbl = &*(*context1).pVtbl;
            let init_vulkan = require_vtbl_fn(vtbl.InitVulkan, "AMFContext1::InitVulkan")?;
            let release = vtbl.Release;

            let r = init_vulkan(context1, device);

            if let Some(release) = release {
                release(context1);
            }

            r
        };

        Error::check(result, "AMFContext1::InitVulkan")
    }
}

impl Clone for Context {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        Self {
            ptr: self.ptr,
            f: self.f,
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct ComponentFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFComponent) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFComponent) -> sys::amf_long,
    init: unsafe extern "C" fn(
        *mut sys::AMFComponent,
        sys::AMF_SURFACE_FORMAT,
        sys::amf_int32,
        sys::amf_int32,
    ) -> sys::AMF_RESULT,
    re_init: unsafe extern "C" fn(
        *mut sys::AMFComponent,
        sys::amf_int32,
        sys::amf_int32,
    ) -> sys::AMF_RESULT,
    terminate: unsafe extern "C" fn(*mut sys::AMFComponent) -> sys::AMF_RESULT,
    drain: unsafe extern "C" fn(*mut sys::AMFComponent) -> sys::AMF_RESULT,
    flush: unsafe extern "C" fn(*mut sys::AMFComponent) -> sys::AMF_RESULT,
    submit_input:
        unsafe extern "C" fn(*mut sys::AMFComponent, *mut sys::AMFData) -> sys::AMF_RESULT,
    query_output:
        unsafe extern "C" fn(*mut sys::AMFComponent, *mut *mut sys::AMFData) -> sys::AMF_RESULT,
    get_context: unsafe extern "C" fn(*mut sys::AMFComponent) -> *mut sys::AMFContext,
}

/// `AMFComponent` の RAII ラッパー
///
/// AMFComponent はスレッドセーフであると AMF ドキュメントに記載されているため、
/// `Send` を実装する。
pub struct Component {
    ptr: *mut sys::AMFComponent,
    f: ComponentFuncs,
}

unsafe impl Send for Component {}

impl Component {
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFComponent,
        acquire: bool,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = ComponentFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFComponent::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFComponent::Release")?,
            init: require_vtbl_fn(vtbl.Init, "AMFComponent::Init")?,
            re_init: require_vtbl_fn(vtbl.ReInit, "AMFComponent::ReInit")?,
            terminate: require_vtbl_fn(vtbl.Terminate, "AMFComponent::Terminate")?,
            drain: require_vtbl_fn(vtbl.Drain, "AMFComponent::Drain")?,
            flush: require_vtbl_fn(vtbl.Flush, "AMFComponent::Flush")?,
            submit_input: require_vtbl_fn(vtbl.SubmitInput, "AMFComponent::SubmitInput")?,
            query_output: require_vtbl_fn(vtbl.QueryOutput, "AMFComponent::QueryOutput")?,
            get_context: require_vtbl_fn(vtbl.GetContext, "AMFComponent::GetContext")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        Ok(Self { ptr, f })
    }

    /// 生ポインタから Component を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は有効な `AMFComponent` を指すこと。
    pub unsafe fn from_raw(ptr: *mut sys::AMFComponent) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false) }
    }

    /// 生ポインタから Acquire 付きで Component を作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFComponent` を指すこと。
    pub unsafe fn from_raw_acquired(ptr: *mut sys::AMFComponent) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true) }
    }

    /// 生ポインタを返す
    pub fn as_ptr(&self) -> *mut sys::AMFComponent {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)
    pub fn into_raw(self) -> *mut sys::AMFComponent {
        let ptr = self.ptr;
        std::mem::forget(self);
        ptr
    }

    /// コンポーネントを初期化する
    pub fn init(
        &self,
        format: sys::AMF_SURFACE_FORMAT,
        width: sys::amf_int32,
        height: sys::amf_int32,
    ) -> sys::AMF_RESULT {
        unsafe { (self.f.init)(self.ptr, format, width, height) }
    }

    /// コンポーネントを再初期化する (解像度変更)
    pub fn re_init(&self, width: sys::amf_int32, height: sys::amf_int32) -> sys::AMF_RESULT {
        unsafe { (self.f.re_init)(self.ptr, width, height) }
    }

    /// コンポーネントを終了する
    pub fn terminate(&self) -> sys::AMF_RESULT {
        unsafe { (self.f.terminate)(self.ptr) }
    }

    /// ドレイン (これ以上の入力なしを通知)
    pub fn drain(&self) -> sys::AMF_RESULT {
        unsafe { (self.f.drain)(self.ptr) }
    }

    /// フラッシュ (内部バッファを破棄)
    pub fn flush(&self) -> sys::AMF_RESULT {
        unsafe { (self.f.flush)(self.ptr) }
    }

    /// データを入力する
    ///
    /// # Safety
    /// `data` は有効な AMFData を指すこと。
    pub unsafe fn submit_input(&self, data: *mut sys::AMFData) -> sys::AMF_RESULT {
        unsafe { (self.f.submit_input)(self.ptr, data) }
    }

    /// 出力データを問い合わせる
    ///
    /// # Safety
    /// `pp_data` は有効な `*mut *mut AMFData` を指すこと。
    pub unsafe fn query_output(&self, pp_data: *mut *mut sys::AMFData) -> sys::AMF_RESULT {
        unsafe { (self.f.query_output)(self.ptr, pp_data) }
    }

    /// コンテキストを取得する (参照カウントは増えない)
    pub fn get_context(&self) -> *mut sys::AMFContext {
        unsafe { (self.f.get_context)(self.ptr) }
    }

    /// AMFPropertyStorage としてアクセスする
    ///
    /// 戻り値の `PropertyStorage` が `Drop` されるまで参照カウントが増える。
    pub fn property_storage(&self) -> Result<PropertyStorage, Error> {
        unsafe { PropertyStorage::from_raw_acquired(self.ptr as *mut sys::AMFPropertyStorage) }
    }
}

impl Clone for Component {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        Self {
            ptr: self.ptr,
            f: self.f,
        }
    }
}

impl Drop for Component {
    fn drop(&mut self) {
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}
