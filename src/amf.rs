//! AMF オブジェクトの RAII ラッパー
//!
//! `*mut AMFSurface` のような生ポインタをラップし、
//! Clone/Drop で `Acquire`/`Release` による参照カウント管理を自動化する。
//! vtable の関数ポインタは初期化時にすべて取り出して構造体メンバーに保持するため、
//! メソッド呼び出しのたびに vtable 検証は行われない。
//!
//! ## 生ポインタ
//!
//! `as_ptr` が返すポインタは、そのハンドルが生存する間のみ有効である。
//! `into_raw` は所有権を放棄して生ポインタを返し、`Release` は呼び出し元が行う。
//! 所有権を放棄したハンドルの `Drop` は `Release` を呼ばない。`into_raw` は
//! ハンドルが保持していた `Context` も手放すため、生ポインタを利用する間は
//! 呼び出し元が `Context` の Clone を保持すること。
//! `as_ptr` / `into_raw` / `get_native` が返す生ポインタ経由の AMF 呼び出しや
//! メモリアクセスは、呼び出し元の責任である。
//!
//! `from_raw` は参照カウントを増やさず、呼び出し元が持つ参照の所有権を引き継ぐ。
//! `from_raw_acquired` は `Acquire` を呼んで参照カウントを 1 増やし、独立した参照を
//! 持つハンドルを作成する。`Surface` / `Buffer` / `PropertyStorage` では、どちらも
//! 新しいハンドル状態を作るため、既存のハンドルとハンドル数を共有しない。
//! 既存のハンドルがある場合は使ってはならない。`Plane` の `from_raw` /
//! `from_raw_acquired` は取得元の `Surface` の状態を共有する。
//!
//! ## フレームデータ
//!
//! `Buffer` / `Plane` の内容は unsafe な `as_slice` / `as_mut_slice` でバイトスライスとして
//! 読み書きできる。対象はホストメモリのオブジェクトに限る。`Plane` のスライスは
//! `hpitch * vpitch` バイトで、各行の先頭は `hpitch` バイト間隔になる。
//! スライスが有効な間は、同じ AMF オブジェクトへのアクセスを他のハンドルから行わないこと。
//! また、返される範囲のメモリが初期化済みであることを呼び出し側が保証すること。
//!
//! ## コンポーネントへの投入
//!
//! コンポーネントに投入する `Surface` / `Buffer` には、投入前にそのオブジェクトを
//! 指す他のハンドルが存在してはならない。`Surface` の場合は `Clone`、`Plane`、
//! `PropertyStorage` が該当する。AMF コンポーネントは投入したオブジェクトを解放する
//! まで排他アクセスするため、他のハンドルからアクセスするとデータ競合になる。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::{Error, require_vtbl_fn};
use crate::sys;
use crate::types::{MemoryType, PlaneType, SurfaceFormat};

// SAFETY: 各型の `Send` / `Sync` の根拠。
// `AMFComponent` は thread-safe と明記されている (AMF API Reference §2.6.1)。
// `AMFPropertyStorage` (§2.2.14.1)、`AMFData` (§2.3.1)、`AMFSurface` (§2.3.3.1) は
// thread-safe ではなく、`AMFPlane` (§2.3.3.3) と `AMFContext` (§2.4.1) には記述がない。
// そのためスレッドセーフでない操作は共有ロックで直列化する。
// 参照カウント操作 (`Acquire` / `Release`) は Clone / Drop で行い、共有ロックしない。
// AMF Wiki の "Guide for Video CODEC Encoder App Developers" §4.2 "AMF Interfaces" に
// "The default implementation of AMFInterface is thread-safe." と明記されている。
// ただし `Context` は除く。詳細は `Context` の Drop 実装に記載している。

/// 同じ AMF オブジェクトを指すハンドルで共有する状態
#[derive(Debug)]
pub(crate) struct AmfObjectState {
    /// AMF オブジェクトのスレッドセーフでない操作を直列化する Mutex
    ///
    /// AMF のインターフェースはほとんどがスレッドセーフではない。同じオブジェクトへの
    /// 同時操作はデータ競合になるため、同じオブジェクトを指すハンドル間でこのロックを
    /// 共有し、操作を 1 つずつ実行する。
    mutex: Mutex<()>,
    /// この状態を共有するハンドルの数
    handles: AtomicUsize,
}

impl AmfObjectState {
    /// ハンドル数 0 の新しい状態を作成する
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            mutex: Mutex::new(()),
            handles: AtomicUsize::new(0),
        })
    }

    /// 共有ロックを取得する
    ///
    /// # Panics
    /// ロックがポイズンされている場合にパニックする。
    pub(crate) fn lock(&self) -> MutexGuard<'_, ()> {
        self.mutex.lock().expect("AMF object lock is poisoned")
    }

    /// ハンドル数を増やす
    fn inc(&self) {
        self.handles.fetch_add(1, Ordering::SeqCst);
    }

    /// ハンドル数を減らす
    fn dec(&self) {
        self.handles.fetch_sub(1, Ordering::SeqCst);
    }

    /// この状態が複数のハンドルで共有されているかを返す
    fn is_shared(&self) -> bool {
        self.handles.load(Ordering::SeqCst) > 1
    }
}

// ---------------------------------------------------------------------------
// PropertyStorage のラッパー
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
    get_property: unsafe extern "C" fn(
        *mut sys::AMFPropertyStorage,
        *const sys::wchar_t,
        *mut sys::AMFVariantStruct,
    ) -> sys::AMF_RESULT,
}

/// `AMFPropertyStorage` の RAII ラッパー
pub struct PropertyStorage {
    ptr: *mut sys::AMFPropertyStorage,
    f: PropertyStorageFuncs,
    state: Arc<AmfObjectState>,
    context: Context,
}

unsafe impl Send for PropertyStorage {}
unsafe impl Sync for PropertyStorage {}

impl PropertyStorage {
    /// ハンドルを作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFPropertyStorage` を指すこと。
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFPropertyStorage,
        acquire: bool,
        state: Arc<AmfObjectState>,
        context: Context,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = PropertyStorageFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFPropertyStorage::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFPropertyStorage::Release")?,
            set_property: require_vtbl_fn(vtbl.SetProperty, "AMFPropertyStorage::SetProperty")?,
            get_property: require_vtbl_fn(vtbl.GetProperty, "AMFPropertyStorage::GetProperty")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        state.inc();
        Ok(Self {
            ptr,
            f,
            state,
            context,
        })
    }

    /// 生ポインタから PropertyStorage を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は `context` に属する有効な `AMFPropertyStorage` を指すこと。
    pub unsafe fn from_raw(
        ptr: *mut sys::AMFPropertyStorage,
        context: Context,
    ) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false, AmfObjectState::new(), context) }
    }

    /// 生ポインタから Acquire 付きで PropertyStorage を作成する
    ///
    /// # Safety
    /// `ptr` は `context` に属する有効な `AMFPropertyStorage` を指すこと。
    pub unsafe fn from_raw_acquired(
        ptr: *mut sys::AMFPropertyStorage,
        context: Context,
    ) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true, AmfObjectState::new(), context) }
    }

    /// このハンドルが保持する `Context` を返す
    pub fn context(&self) -> Context {
        self.context.clone()
    }

    /// 生ポインタを返す。
    pub fn as_ptr(&self) -> *mut sys::AMFPropertyStorage {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)。
    pub fn into_raw(mut self) -> *mut sys::AMFPropertyStorage {
        let ptr = self.ptr;
        self.ptr = std::ptr::null_mut();
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
        let _guard = self.state.lock();
        unsafe { (self.f.set_property)(self.ptr, name, value) }
    }

    /// Int64 プロパティを設定する
    pub fn set_property_int64(&self, name: &str, value: i64) -> Result<(), Error> {
        let name_w = sys::to_wstring(name);
        let var = sys::AMFVariantStruct::from_int64(value);
        let result = unsafe { self.set_property(name_w.as_ptr(), var) };
        Error::check(result, format!("SetProperty({name})"))
    }

    /// Size プロパティを設定する
    pub fn set_property_size(&self, name: &str, width: i32, height: i32) -> Result<(), Error> {
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
        let _guard = self.state.lock();
        unsafe { (self.f.get_property)(self.ptr, name, p_value) }
    }

    /// Int64 プロパティを取得する
    ///
    /// # Errors
    ///
    /// プロパティを取得できない場合、または値が Int64 でない場合にエラーを返す。
    pub fn get_property_int64(&self, name: &str) -> Result<i64, Error> {
        let name_w = sys::to_wstring(name);
        let mut value = Variant::empty();
        let result = unsafe { self.get_property(name_w.as_ptr(), value.as_ptr()) };
        Error::check(result, format!("GetProperty({name})"))?;
        value.as_int64().ok_or_else(|| {
            Error::new_custom(
                "AMFPropertyStorage::GetProperty",
                &format!("property {name} is not int64"),
            )
        })
    }

    /// Size プロパティを取得する
    ///
    /// 幅と高さの組を返す。
    ///
    /// # Errors
    ///
    /// プロパティを取得できない場合、または値が Size でない場合にエラーを返す。
    pub fn get_property_size(&self, name: &str) -> Result<(i32, i32), Error> {
        let name_w = sys::to_wstring(name);
        let mut value = Variant::empty();
        let result = unsafe { self.get_property(name_w.as_ptr(), value.as_ptr()) };
        Error::check(result, format!("GetProperty({name})"))?;
        value.as_size().ok_or_else(|| {
            Error::new_custom(
                "AMFPropertyStorage::GetProperty",
                &format!("property {name} is not size"),
            )
        })
    }
}

impl Clone for PropertyStorage {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        self.state.inc();
        Self {
            ptr: self.ptr,
            f: self.f,
            state: Arc::clone(&self.state),
            context: self.context.clone(),
        }
    }
}

impl Drop for PropertyStorage {
    fn drop(&mut self) {
        self.state.dec();
        if self.ptr.is_null() {
            return;
        }
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// Surface のラッパー
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct SurfaceFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFSurface) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFSurface) -> sys::amf_long,
    set_pts: unsafe extern "C" fn(*mut sys::AMFSurface, sys::amf_pts),
    get_pts: unsafe extern "C" fn(*mut sys::AMFSurface) -> sys::amf_pts,
    set_duration: unsafe extern "C" fn(*mut sys::AMFSurface, sys::amf_pts),
    get_plane:
        unsafe extern "C" fn(*mut sys::AMFSurface, sys::AMF_PLANE_TYPE) -> *mut sys::AMFPlane,
    get_plane_at: unsafe extern "C" fn(*mut sys::AMFSurface, sys::amf_size) -> *mut sys::AMFPlane,
    get_memory_type: unsafe extern "C" fn(*mut sys::AMFSurface) -> sys::AMF_MEMORY_TYPE,
    convert: unsafe extern "C" fn(*mut sys::AMFSurface, sys::AMF_MEMORY_TYPE) -> sys::AMF_RESULT,
}

/// `AMFSurface` の RAII ラッパー
///
/// `get_plane()` / `get_plane_at()` は `Plane` を返し、
/// `property_storage()` が返す `PropertyStorage` は同じ AMF オブジェクトを指す。
#[derive(Debug)]
pub struct Surface {
    ptr: *mut sys::AMFSurface,
    f: SurfaceFuncs,
    state: Arc<AmfObjectState>,
    /// 生存している Plane ハンドルの数
    ///
    /// `Surface` の `Clone` 間で共有する。`Plane` の生成・`Clone` で増え、drop で減る。
    planes: Arc<AtomicUsize>,
    context: Context,
}

unsafe impl Send for Surface {}
unsafe impl Sync for Surface {}

impl Surface {
    /// ハンドルを作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFSurface` を指すこと。
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFSurface,
        acquire: bool,
        state: Arc<AmfObjectState>,
        context: Context,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = SurfaceFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFSurface::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFSurface::Release")?,
            set_pts: require_vtbl_fn(vtbl.SetPts, "AMFSurface::SetPts")?,
            get_pts: require_vtbl_fn(vtbl.GetPts, "AMFSurface::GetPts")?,
            set_duration: require_vtbl_fn(vtbl.SetDuration, "AMFSurface::SetDuration")?,
            get_plane: require_vtbl_fn(vtbl.GetPlane, "AMFSurface::GetPlane")?,
            get_plane_at: require_vtbl_fn(vtbl.GetPlaneAt, "AMFSurface::GetPlaneAt")?,
            get_memory_type: require_vtbl_fn(vtbl.GetMemoryType, "AMFSurface::GetMemoryType")?,
            convert: require_vtbl_fn(vtbl.Convert, "AMFData::Convert")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        state.inc();
        Ok(Self {
            ptr,
            f,
            state,
            planes: Arc::new(AtomicUsize::new(0)),
            context,
        })
    }

    /// 生ポインタから Surface を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は `context` から取得した有効な `AMFSurface` を指すこと。
    pub unsafe fn from_raw(ptr: *mut sys::AMFSurface, context: Context) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false, AmfObjectState::new(), context) }
    }

    /// 生ポインタから Acquire 付きで Surface を作成する
    ///
    /// # Safety
    /// `ptr` は `context` から取得した有効な `AMFSurface` を指すこと。
    pub unsafe fn from_raw_acquired(
        ptr: *mut sys::AMFSurface,
        context: Context,
    ) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true, AmfObjectState::new(), context) }
    }

    /// このハンドルが他のハンドルと共有されているかを返す
    pub(crate) fn is_shared(&self) -> bool {
        self.state.is_shared()
    }

    /// 生存している Plane ハンドルの数を増やす
    fn inc_plane(&self) {
        self.planes.fetch_add(1, Ordering::SeqCst);
    }

    /// 生存している Plane ハンドルの数を減らす
    fn dec_plane(&self) {
        self.planes.fetch_sub(1, Ordering::SeqCst);
    }

    /// 生存している Plane ハンドルがあるかを返す
    fn has_planes(&self) -> bool {
        self.planes.load(Ordering::SeqCst) > 0
    }

    /// このハンドルが保持する `Context` を返す
    pub fn context(&self) -> Context {
        self.context.clone()
    }

    /// 生ポインタを返す。
    pub fn as_ptr(&self) -> *mut sys::AMFSurface {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)。
    pub fn into_raw(mut self) -> *mut sys::AMFSurface {
        let ptr = self.ptr;
        self.ptr = std::ptr::null_mut();
        ptr
    }

    /// PTS (100 ナノ秒単位のタイムスタンプ) を設定する
    pub fn set_pts(&self, pts: i64) {
        let _guard = self.state.lock();
        unsafe { (self.f.set_pts)(self.ptr, pts) }
    }

    /// PTS (100 ナノ秒単位のタイムスタンプ) を取得する
    pub fn get_pts(&self) -> i64 {
        let _guard = self.state.lock();
        unsafe { (self.f.get_pts)(self.ptr) }
    }

    /// このサーフェスのメモリ種別を返す
    pub fn get_memory_type(&self) -> MemoryType {
        let _guard = self.state.lock();
        let memory_type = unsafe { (self.f.get_memory_type)(self.ptr) };
        MemoryType::from_amf(memory_type)
    }

    /// Duration (100 ナノ秒単位) を設定する
    pub fn set_duration(&self, duration: i64) {
        let _guard = self.state.lock();
        unsafe { (self.f.set_duration)(self.ptr, duration) }
    }

    /// 指定プレーンを取得する
    ///
    /// # Errors
    /// 指定したプレーンを取得できない場合にエラーを返す。
    pub fn get_plane(&self, plane_type: PlaneType) -> Result<Plane<'_>, Error> {
        let _guard = self.state.lock();
        let plane = unsafe { (self.f.get_plane)(self.ptr, plane_type.to_amf()) };
        if plane.is_null() {
            return Err(Error::new_custom("Surface::get_plane", "plane is null"));
        }
        unsafe { Plane::from_raw_with_acquire(plane, true, self) }
    }

    /// インデックスでプレーンを取得する
    ///
    /// # Errors
    /// `index` に対応するプレーンを取得できない場合にエラーを返す。
    pub fn get_plane_at(&self, index: usize) -> Result<Plane<'_>, Error> {
        let _guard = self.state.lock();
        let plane = unsafe { (self.f.get_plane_at)(self.ptr, index) };
        if plane.is_null() {
            return Err(Error::new_custom("Surface::get_plane_at", "plane is null"));
        }
        unsafe { Plane::from_raw_with_acquire(plane, true, self) }
    }

    /// メモリ種別を変換する
    ///
    /// # Errors
    /// [`Plane`] が生存している場合、または AMF の変換に失敗した場合にエラーを返す。
    pub fn convert(&self, memory_type: MemoryType) -> Result<(), Error> {
        let _guard = self.state.lock();
        if self.has_planes() {
            return Err(Error::new_custom(
                "Surface::convert",
                "planes are alive; drop planes before converting",
            ));
        }
        Error::check(
            unsafe { (self.f.convert)(self.ptr, memory_type.to_amf()) },
            "AMFData::Convert",
        )
    }

    /// AMFPropertyStorage としてアクセスする
    pub fn property_storage(&self) -> Result<PropertyStorage, Error> {
        unsafe {
            PropertyStorage::from_raw_with_acquire(
                self.ptr as *mut sys::AMFPropertyStorage,
                true,
                Arc::clone(&self.state),
                self.context.clone(),
            )
        }
    }
}

impl Clone for Surface {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        self.state.inc();
        Self {
            ptr: self.ptr,
            f: self.f,
            state: Arc::clone(&self.state),
            planes: Arc::clone(&self.planes),
            context: self.context.clone(),
        }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        self.state.dec();
        if self.ptr.is_null() {
            return;
        }
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// Buffer のラッパー
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct BufferFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_long,
    get_native: unsafe extern "C" fn(*mut sys::AMFBuffer) -> *mut std::ffi::c_void,
    get_size: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_size,
    set_size: unsafe extern "C" fn(*mut sys::AMFBuffer, sys::amf_size) -> sys::AMF_RESULT,
    set_pts: unsafe extern "C" fn(*mut sys::AMFBuffer, sys::amf_pts),
    get_pts: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_pts,
    get_duration: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::amf_pts,
    get_memory_type: unsafe extern "C" fn(*mut sys::AMFBuffer) -> sys::AMF_MEMORY_TYPE,
}

/// `AMFBuffer` の RAII ラッパー
#[derive(Debug)]
pub struct Buffer {
    ptr: *mut sys::AMFBuffer,
    f: BufferFuncs,
    state: Arc<AmfObjectState>,
    context: Context,
}

unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}

impl Buffer {
    /// ハンドルを作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFBuffer` を指すこと。
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFBuffer,
        acquire: bool,
        state: Arc<AmfObjectState>,
        context: Context,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = BufferFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFBuffer::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFBuffer::Release")?,
            get_native: require_vtbl_fn(vtbl.GetNative, "AMFBuffer::GetNative")?,
            get_size: require_vtbl_fn(vtbl.GetSize, "AMFBuffer::GetSize")?,
            set_size: require_vtbl_fn(vtbl.SetSize, "AMFBuffer::SetSize")?,
            set_pts: require_vtbl_fn(vtbl.SetPts, "AMFBuffer::SetPts")?,
            get_pts: require_vtbl_fn(vtbl.GetPts, "AMFBuffer::GetPts")?,
            get_duration: require_vtbl_fn(vtbl.GetDuration, "AMFBuffer::GetDuration")?,
            get_memory_type: require_vtbl_fn(vtbl.GetMemoryType, "AMFBuffer::GetMemoryType")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        state.inc();
        Ok(Self {
            ptr,
            f,
            state,
            context,
        })
    }

    /// 生ポインタから Buffer を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は `context` から取得した有効な `AMFBuffer` を指すこと。
    pub unsafe fn from_raw(ptr: *mut sys::AMFBuffer, context: Context) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false, AmfObjectState::new(), context) }
    }

    /// 生ポインタから Acquire 付きで Buffer を作成する
    ///
    /// # Safety
    /// `ptr` は `context` から取得した有効な `AMFBuffer` を指すこと。
    pub unsafe fn from_raw_acquired(
        ptr: *mut sys::AMFBuffer,
        context: Context,
    ) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true, AmfObjectState::new(), context) }
    }

    /// このハンドルが他のハンドルと共有されているかを返す
    pub(crate) fn is_shared(&self) -> bool {
        self.state.is_shared()
    }

    /// このハンドルが保持する `Context` を返す
    pub fn context(&self) -> Context {
        self.context.clone()
    }

    /// 生ポインタを返す。
    pub fn as_ptr(&self) -> *mut sys::AMFBuffer {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)。
    pub fn into_raw(mut self) -> *mut sys::AMFBuffer {
        let ptr = self.ptr;
        self.ptr = std::ptr::null_mut();
        ptr
    }

    /// バッファのネイティブポインタを返す
    ///
    /// 返されるポインタはこのハンドルが生存している間だけ有効である。
    /// ポインタ経由の読み書きは呼び出し元の責任であり、Rust のスライスとして
    /// 扱う場合は [`Buffer::as_slice`] / [`Buffer::as_mut_slice`] を使う。
    pub fn get_native(&self) -> *mut std::ffi::c_void {
        let _guard = self.state.lock();
        unsafe { (self.f.get_native)(self.ptr) }
    }

    /// バッファのサイズ (バイト数) を返す
    pub fn get_size(&self) -> usize {
        let _guard = self.state.lock();
        unsafe { (self.f.get_size)(self.ptr) }
    }

    /// このバッファのメモリ種別を返す
    pub fn get_memory_type(&self) -> MemoryType {
        let _guard = self.state.lock();
        let memory_type = unsafe { (self.f.get_memory_type)(self.ptr) };
        MemoryType::from_amf(memory_type)
    }

    /// バッファのサイズ (バイト数) を設定する
    ///
    /// 設定できる最大サイズは確保済みのサイズである。[`Buffer::get_size`] は
    /// 最後に設定に成功したサイズを返す。
    ///
    /// # Errors
    /// 指定したサイズを設定できない場合にエラーを返す。
    pub fn set_size(&self, size: usize) -> Result<(), Error> {
        let _guard = self.state.lock();
        Error::check(
            unsafe { (self.f.set_size)(self.ptr, size) },
            "AMFBuffer::SetSize",
        )
    }

    /// バッファがホストメモリにあることを確認する
    fn check_host_memory(&self, function: &'static str) -> Result<(), Error> {
        let memory_type = unsafe { (self.f.get_memory_type)(self.ptr) };
        if memory_type != sys::AMF_MEMORY_TYPE::AMF_MEMORY_HOST {
            return Err(Error::new_custom(function, "buffer is not in host memory"));
        }
        Ok(())
    }

    /// ホストメモリのバッファの内容をバイトスライスとして返す
    ///
    /// 返されるスライスの長さは [`Buffer::get_size`] が返すサイズと同じである。
    /// サイズが 0 の場合は空のスライスを返す。
    ///
    /// # Safety
    ///
    /// 次の条件をすべて満たすこと。
    ///
    /// - 返される範囲のメモリが初期化済みであること
    /// - このスライスが有効な間、同じ AMF オブジェクトへのアクセスを
    ///   他のハンドルから行わないこと
    /// - このスライスが有効な間、同じ AMF オブジェクトのサイズ変更や解放を
    ///   行わないこと
    ///
    /// # Errors
    /// バッファの内容を Rust のスライスとして扱えない場合にエラーを返す。
    pub unsafe fn as_slice(&self) -> Result<&[u8], Error> {
        let _guard = self.state.lock();
        self.check_host_memory("Buffer::as_slice")?;
        let ptr = unsafe { (self.f.get_native)(self.ptr) } as *const u8;
        let size = unsafe { (self.f.get_size)(self.ptr) };
        if size == 0 {
            return Ok(&[]);
        }
        validate_slice_ptr("Buffer::as_slice", ptr)?;
        validate_slice_len("Buffer::as_slice", size)?;
        // SAFETY: ptr は null でなく、size はスライスの長さとして有効な範囲にあり、
        // バッファは self が生きている間有効である。
        Ok(unsafe { std::slice::from_raw_parts(ptr, size) })
    }

    /// ホストメモリのバッファの内容を書き込み用のバイトスライスとして返す
    ///
    /// 返されるスライスの長さは [`Buffer::get_size`] が返すサイズと同じである。
    /// サイズが 0 の場合は空のスライスを返す。
    ///
    /// # Safety
    ///
    /// 次の条件をすべて満たすこと。
    ///
    /// - 返される範囲のメモリが初期化済みであること
    /// - このスライスが有効な間、同じメモリを指す他の参照やハンドルが
    ///   存在しないこと
    /// - このスライスが有効な間、同じ AMF オブジェクトへのアクセスや
    ///   サイズ変更、解放を行わないこと
    ///
    /// # Errors
    /// バッファの内容を Rust のスライスとして扱えない場合にエラーを返す。
    pub unsafe fn as_mut_slice(&mut self) -> Result<&mut [u8], Error> {
        let _guard = self.state.lock();
        self.check_host_memory("Buffer::as_mut_slice")?;
        let ptr = unsafe { (self.f.get_native)(self.ptr) } as *mut u8;
        let size = unsafe { (self.f.get_size)(self.ptr) };
        if size == 0 {
            return Ok(&mut []);
        }
        validate_slice_ptr("Buffer::as_mut_slice", ptr)?;
        validate_slice_len("Buffer::as_mut_slice", size)?;
        // SAFETY: ptr は null でなく、size はスライスの長さとして有効な範囲にあり、
        // バッファは self が生きている間有効である。
        Ok(unsafe { std::slice::from_raw_parts_mut(ptr, size) })
    }

    /// PTS (100 ナノ秒単位のタイムスタンプ) を設定する
    pub fn set_pts(&self, pts: i64) {
        let _guard = self.state.lock();
        unsafe { (self.f.set_pts)(self.ptr, pts) }
    }

    /// PTS (100 ナノ秒単位のタイムスタンプ) を取得する
    pub fn get_pts(&self) -> i64 {
        let _guard = self.state.lock();
        unsafe { (self.f.get_pts)(self.ptr) }
    }

    /// Duration (100 ナノ秒単位) を取得する
    pub fn get_duration(&self) -> i64 {
        let _guard = self.state.lock();
        unsafe { (self.f.get_duration)(self.ptr) }
    }

    /// AMFPropertyStorage としてアクセスする
    pub fn property_storage(&self) -> Result<PropertyStorage, Error> {
        unsafe {
            PropertyStorage::from_raw_with_acquire(
                self.ptr as *mut sys::AMFPropertyStorage,
                true,
                Arc::clone(&self.state),
                self.context.clone(),
            )
        }
    }
}

impl Clone for Buffer {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        self.state.inc();
        Self {
            ptr: self.ptr,
            f: self.f,
            state: Arc::clone(&self.state),
            context: self.context.clone(),
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        self.state.dec();
        if self.ptr.is_null() {
            return;
        }
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// Plane のラッパー
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
struct PlaneFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_long,
    get_native: unsafe extern "C" fn(*mut sys::AMFPlane) -> *mut std::ffi::c_void,
    get_hpitch: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_int32,
    get_vpitch: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_int32,
    get_width: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_int32,
    get_height: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_int32,
    is_tiled: unsafe extern "C" fn(*mut sys::AMFPlane) -> sys::amf_bool,
}

/// `AMFPlane` の RAII ラッパー
#[derive(Debug)]
pub struct Plane<'a> {
    ptr: *mut sys::AMFPlane,
    f: PlaneFuncs,
    // Surface.h の GetPlanesCount / GetPlaneAt / GetPlane の
    // "do not store planes outside. should be used together with Surface" に従い、
    // Plane の寿命を Surface に縛る。
    /// 取得元の `Surface`
    surface: &'a Surface,
}

// SAFETY: プレーンの情報を取得するメソッドは `Surface` と共有するロックで直列化され、
// 参照カウント操作は AMFInterface が thread-safe であるため、複数スレッドから使える。
unsafe impl<'a> Send for Plane<'a> {}
unsafe impl<'a> Sync for Plane<'a> {}

impl<'a> Plane<'a> {
    /// ハンドルを作成する
    ///
    /// # Safety
    /// `ptr` は null でなく、`surface` が持つ有効な `AMFPlane` を指すこと。
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFPlane,
        acquire: bool,
        surface: &'a Surface,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = PlaneFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFPlane::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFPlane::Release")?,
            get_native: require_vtbl_fn(vtbl.GetNative, "AMFPlane::GetNative")?,
            get_hpitch: require_vtbl_fn(vtbl.GetHPitch, "AMFPlane::GetHPitch")?,
            get_vpitch: require_vtbl_fn(vtbl.GetVPitch, "AMFPlane::GetVPitch")?,
            get_width: require_vtbl_fn(vtbl.GetWidth, "AMFPlane::GetWidth")?,
            get_height: require_vtbl_fn(vtbl.GetHeight, "AMFPlane::GetHeight")?,
            is_tiled: require_vtbl_fn(vtbl.IsTiled, "AMFPlane::IsTiled")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        surface.state.inc();
        surface.inc_plane();
        Ok(Self { ptr, f, surface })
    }

    /// 生ポインタから Plane を作成する (Acquire なし)
    ///
    /// # Safety
    /// `ptr` は `surface` が持つ有効な `AMFPlane` を指すこと。`ptr` は null であってはならない。
    /// 同じ参照を二重に所有しないこと (`from_raw` を同じ参照に対して複数回呼ばないこと)。
    /// `as_ptr` が返した借用ポインタを渡してはならない。`surface` の `convert` と並行して
    /// 呼ばないこと。
    ///
    /// # Errors
    /// `ptr` から `Plane` を作成できない場合にエラーを返す。
    pub unsafe fn from_raw(ptr: *mut sys::AMFPlane, surface: &'a Surface) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false, surface) }
    }

    /// 生ポインタから Acquire 付きで Plane を作成する
    ///
    /// # Safety
    /// `ptr` は `surface` が持つ有効な `AMFPlane` を指すこと。`ptr` は null であってはならない。
    /// 新たに `Acquire` するため、既存のハンドルと並存してよい。`surface` の `convert` と
    /// 並行して呼ばないこと。
    ///
    /// # Errors
    /// `ptr` から `Plane` を作成できない場合にエラーを返す。
    pub unsafe fn from_raw_acquired(
        ptr: *mut sys::AMFPlane,
        surface: &'a Surface,
    ) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, true, surface) }
    }

    /// 取得元の `Surface` が保持する `Context` を返す
    pub fn context(&self) -> Context {
        self.surface.context()
    }

    /// 生ポインタを返す。
    pub fn as_ptr(&self) -> *mut sys::AMFPlane {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)。
    ///
    /// 返した生ポインタを利用する間は、取得元の `Surface` を破棄せず、
    /// `Surface::convert` も呼ばないこと。
    pub fn into_raw(mut self) -> *mut sys::AMFPlane {
        let ptr = self.ptr;
        self.ptr = std::ptr::null_mut();
        ptr
    }

    /// プレーンのネイティブポインタを返す
    ///
    /// 返されるポインタは取得元の `Surface` が生存している間だけ有効である。
    /// ポインタ経由の読み書きは呼び出し元の責任であり、Rust のスライスとして
    /// 扱う場合は [`Plane::as_slice`] / [`Plane::as_mut_slice`] を使う。
    pub fn get_native(&self) -> *mut std::ffi::c_void {
        let _guard = self.surface.state.lock();
        unsafe { (self.f.get_native)(self.ptr) }
    }

    /// 水平ピッチ (行あたりのバイト数) を取得する
    ///
    /// 行の先頭から次の行の先頭までのバイト数であり、幅 (画素数) とは異なる。
    pub fn get_hpitch(&self) -> i32 {
        let _guard = self.surface.state.lock();
        unsafe { (self.f.get_hpitch)(self.ptr) }
    }

    /// 垂直ピッチ (行数) を取得する
    pub fn get_vpitch(&self) -> i32 {
        let _guard = self.surface.state.lock();
        unsafe { (self.f.get_vpitch)(self.ptr) }
    }

    /// 幅を取得する
    ///
    /// crop region の幅をピクセル単位で返す。crop region が未設定の場合は、
    /// サーフェスの幅とピクセルフォーマットに基づく。
    pub fn get_width(&self) -> i32 {
        let _guard = self.surface.state.lock();
        unsafe { (self.f.get_width)(self.ptr) }
    }

    /// 高さを取得する
    ///
    /// crop region の高さをピクセル単位で返す。crop region が未設定の場合は、
    /// サーフェスの高さとピクセルフォーマットに基づく。
    pub fn get_height(&self) -> i32 {
        let _guard = self.surface.state.lock();
        unsafe { (self.f.get_height)(self.ptr) }
    }

    /// 物理メモリがタイル状に並んでいるかを返す
    ///
    /// `true` の場合、メモリは連続したスキャンラインとしてアクセスできない。
    /// タイル状のプレーンは [`Plane::as_slice`] / [`Plane::as_mut_slice`] の
    /// 対象にならない。
    pub fn is_tiled(&self) -> bool {
        let _guard = self.surface.state.lock();
        unsafe { (self.f.is_tiled)(self.ptr) != 0 }
    }

    /// 取得元の `Surface` がホストメモリにあることを確認する
    fn check_host_memory(&self, function: &'static str) -> Result<(), Error> {
        let memory_type = unsafe { (self.surface.f.get_memory_type)(self.surface.ptr) };
        if memory_type != sys::AMF_MEMORY_TYPE::AMF_MEMORY_HOST {
            return Err(Error::new_custom(function, "surface is not in host memory"));
        }
        Ok(())
    }

    /// ホストメモリにあり、タイル状でないプレーンのメモリをバイトスライスとして返す
    ///
    /// 返されるスライスの長さは `get_hpitch() * get_vpitch()` バイトである。
    /// 各行の先頭は [`Plane::get_hpitch`] バイト間隔で並び、幅方向の有効な
    /// バイト数は [`Plane::get_width`] とピクセルフォーマットから決まる。
    ///
    /// # Safety
    ///
    /// 次の条件をすべて満たすこと。
    ///
    /// - 返される範囲のメモリが初期化済みであること
    /// - このスライスが有効な間、取得元の `Surface` やプレーンへのアクセスを
    ///   他のハンドルから行わないこと
    /// - このスライスが有効な間、取得元の `Surface` の変換や解放を行わないこと
    ///
    /// # Errors
    /// プレーンの内容を Rust のスライスとして扱えない場合にエラーを返す。
    pub unsafe fn as_slice(&self) -> Result<&[u8], Error> {
        let _guard = self.surface.state.lock();
        self.check_host_memory("Plane::as_slice")?;
        let ptr = unsafe { (self.f.get_native)(self.ptr) } as *const u8;
        let hpitch = unsafe { (self.f.get_hpitch)(self.ptr) };
        let vpitch = unsafe { (self.f.get_vpitch)(self.ptr) };
        let is_tiled = unsafe { (self.f.is_tiled)(self.ptr) } != 0;
        let size = plane_slice_len("Plane::as_slice", is_tiled, hpitch, vpitch)?;
        validate_slice_ptr("Plane::as_slice", ptr)?;
        // SAFETY: ptr は null でなく、size は hpitch と vpitch から計算した
        // スライスの長さとして有効な範囲にあり、プレーンは Surface が生きている間有効である。
        Ok(unsafe { std::slice::from_raw_parts(ptr, size) })
    }

    /// ホストメモリにあり、タイル状でないプレーンのメモリを書き込み用のバイトスライスとして返す
    ///
    /// 返されるスライスの長さは `get_hpitch() * get_vpitch()` バイトである。
    /// 各行の先頭は [`Plane::get_hpitch`] バイト間隔で並び、幅方向の有効な
    /// バイト数は [`Plane::get_width`] とピクセルフォーマットから決まる。
    ///
    /// # Safety
    ///
    /// 次の条件をすべて満たすこと。
    ///
    /// - 返される範囲のメモリが初期化済みであること
    /// - このスライスが有効な間、同じメモリを指す他の参照やハンドルが
    ///   存在しないこと
    /// - このスライスが有効な間、取得元の `Surface` やプレーンへのアクセス、
    ///   変換、解放を行わないこと
    ///
    /// # Errors
    /// プレーンの内容を Rust のスライスとして扱えない場合にエラーを返す。
    pub unsafe fn as_mut_slice(&mut self) -> Result<&mut [u8], Error> {
        let _guard = self.surface.state.lock();
        self.check_host_memory("Plane::as_mut_slice")?;
        let ptr = unsafe { (self.f.get_native)(self.ptr) } as *mut u8;
        let hpitch = unsafe { (self.f.get_hpitch)(self.ptr) };
        let vpitch = unsafe { (self.f.get_vpitch)(self.ptr) };
        let is_tiled = unsafe { (self.f.is_tiled)(self.ptr) } != 0;
        let size = plane_slice_len("Plane::as_mut_slice", is_tiled, hpitch, vpitch)?;
        validate_slice_ptr("Plane::as_mut_slice", ptr)?;
        // SAFETY: ptr は null でなく、size は hpitch と vpitch から計算した
        // スライスの長さとして有効な範囲にあり、プレーンは Surface が生きている間有効である。
        Ok(unsafe { std::slice::from_raw_parts_mut(ptr, size) })
    }
}

impl Clone for Plane<'_> {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        self.surface.state.inc();
        self.surface.inc_plane();
        Self {
            ptr: self.ptr,
            f: self.f,
            surface: self.surface,
        }
    }
}

impl Drop for Plane<'_> {
    fn drop(&mut self) {
        self.surface.state.dec();
        if !self.ptr.is_null() {
            unsafe {
                (self.f.release)(self.ptr);
            }
        }
        // release より先に dec_plane すると、release の完了前に convert が Plane 数 0 を
        // 観測して AMF Convert を実行しうるため、release 後に減らす。
        self.surface.dec_plane();
    }
}

/// スライスの生成に使うポインタが null でないことを検証する
fn validate_slice_ptr(function: &'static str, ptr: *const u8) -> Result<(), Error> {
    if ptr.is_null() {
        return Err(Error::new_custom(function, "native pointer is null"));
    }
    Ok(())
}

/// スライスの長さが Rust のスライスの不変条件を満たすことを検証する
fn validate_slice_len(function: &'static str, len: usize) -> Result<(), Error> {
    if len > isize::MAX as usize {
        return Err(Error::new_custom(
            function,
            "slice length exceeds isize::MAX",
        ));
    }
    Ok(())
}

/// プレーンのレイアウトを検証し、スライスの長さを計算する
fn plane_slice_len(
    function: &'static str,
    is_tiled: bool,
    hpitch: i32,
    vpitch: i32,
) -> Result<usize, Error> {
    if is_tiled {
        return Err(Error::new_custom(function, "plane is tiled"));
    }
    let hpitch =
        usize::try_from(hpitch).map_err(|_| Error::new_custom(function, "hpitch is negative"))?;
    let vpitch =
        usize::try_from(vpitch).map_err(|_| Error::new_custom(function, "vpitch is negative"))?;
    checked_mul_len(function, hpitch, vpitch)
}

/// 2 つの長さの積をスライスの長さとして計算する
fn checked_mul_len(function: &'static str, a: usize, b: usize) -> Result<usize, Error> {
    let len = a
        .checked_mul(b)
        .ok_or_else(|| Error::new_custom(function, "plane size overflows usize"))?;
    validate_slice_len(function, len)?;
    Ok(len)
}

// ---------------------------------------------------------------------------
// Context のラッパー
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct ContextFuncs {
    acquire: unsafe extern "C" fn(*mut sys::AMFContext) -> sys::amf_long,
    release: unsafe extern "C" fn(*mut sys::AMFContext) -> sys::amf_long,
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

/// `AMFContext` を共有するハンドル
///
/// 新しい Context は [`crate::AmfLibrary::create_context`] で作成する。`Clone` は
/// 同じ `AMFContext` を指すハンドルを返す。最後のハンドルが破棄されると
/// `AMFContext` は破棄される。
///
/// 同じ `AMFContext` を指すハンドル同士は等しいと判定される。
pub struct Context {
    ptr: *mut sys::AMFContext,
    f: ContextFuncs,
    state: Arc<AmfObjectState>,
}

unsafe impl Send for Context {}
unsafe impl Sync for Context {}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Context")
            .field("ptr", &format_args!("{:p}", self.ptr))
            .finish()
    }
}

impl PartialEq for Context {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}

impl Eq for Context {}

impl Context {
    /// 共有状態を指定して生ポインタから Context を作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFContext` を指すこと。呼び出し元の参照の所有権を引き継ぐ。
    pub(crate) unsafe fn from_raw_with_state(
        ptr: *mut sys::AMFContext,
        state: Arc<AmfObjectState>,
    ) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let f = ContextFuncs {
            acquire: require_vtbl_fn(vtbl.Acquire, "AMFContext::Acquire")?,
            release: require_vtbl_fn(vtbl.Release, "AMFContext::Release")?,
            alloc_surface: require_vtbl_fn(vtbl.AllocSurface, "AMFContext::AllocSurface")?,
            alloc_buffer: require_vtbl_fn(vtbl.AllocBuffer, "AMFContext::AllocBuffer")?,
            query_interface: require_vtbl_fn(vtbl.QueryInterface, "AMFContext::QueryInterface")?,
        };
        state.inc();
        Ok(Self { ptr, f, state })
    }

    /// このハンドルが指す `AMFContext` の生ポインタを返す。
    pub fn as_ptr(&self) -> *mut sys::AMFContext {
        self.ptr
    }

    /// Surface を確保する
    ///
    /// # Errors
    /// Surface を確保できない場合にエラーを返す。
    pub fn alloc_surface(
        &self,
        memory_type: MemoryType,
        format: SurfaceFormat,
        width: i32,
        height: i32,
    ) -> Result<Surface, Error> {
        let surface = {
            let _guard = self.state.lock();
            let mut surface: *mut sys::AMFSurface = std::ptr::null_mut();
            let result = unsafe {
                (self.f.alloc_surface)(
                    self.ptr,
                    memory_type.to_amf(),
                    format.to_amf(),
                    width,
                    height,
                    &mut surface,
                )
            };
            Error::check(result, "AMFContext::AllocSurface")?;

            if surface.is_null() {
                return Err(Error::new_custom(
                    "Context::alloc_surface",
                    "AllocSurface returned null",
                ));
            }
            surface
        };

        unsafe {
            Surface::from_raw_with_acquire(surface, false, AmfObjectState::new(), self.clone())
        }
    }

    /// Buffer を確保する
    ///
    /// # Errors
    /// Buffer を確保できない場合にエラーを返す。
    pub fn alloc_buffer(&self, memory_type: MemoryType, size: usize) -> Result<Buffer, Error> {
        let buffer = {
            let _guard = self.state.lock();
            let mut buffer: *mut sys::AMFBuffer = std::ptr::null_mut();
            let result =
                unsafe { (self.f.alloc_buffer)(self.ptr, memory_type.to_amf(), size, &mut buffer) };
            Error::check(result, "AMFContext::AllocBuffer")?;

            if buffer.is_null() {
                return Err(Error::new_custom(
                    "Context::alloc_buffer",
                    "AllocBuffer returned null",
                ));
            }
            buffer
        };

        unsafe { Buffer::from_raw_with_acquire(buffer, false, AmfObjectState::new(), self.clone()) }
    }

    /// Vulkan デバイスを初期化する
    ///
    /// `device` が `null` の場合はデフォルトデバイスが使用される。
    ///
    /// # Safety
    /// `device` は有効な Vulkan デバイスを指すこと。
    pub(crate) unsafe fn init_vulkan(&self, device: *mut std::ffi::c_void) -> Result<(), Error> {
        let _guard = self.state.lock();

        let mut context1_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
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
        // AMFContext は AMFInterface の参照カウントで管理されている。
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        self.state.inc();
        Self {
            ptr: self.ptr,
            f: self.f,
            state: Arc::clone(&self.state),
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // 最後の参照が解放されると AMFContext のデストラクタが Terminate に相当する処理を行う。
        // ドキュメント上、Release の呼び出しはスレッドセーフであるが、
        // 実測ベースではスレッドセーフではなかったため、ロックを取って解放する。
        let _guard = self.state.lock();
        unsafe {
            (self.f.release)(self.ptr);
        }
        self.state.dec();
    }
}

// ---------------------------------------------------------------------------
// Component のラッパー
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
    get_caps:
        unsafe extern "C" fn(*mut sys::AMFComponent, *mut *mut sys::AMFCaps) -> sys::AMF_RESULT,
}

/// `AMFComponent` の RAII ラッパー
///
/// # スレッド安全性について
///
/// `AMFComponent` は、ドキュメント上はスレッドセーフと記載されているが、実際はスレッドセーフではない。
/// 少なくとも `AMFComponent::Init()` および `AMFComponent::ReInit()` では、グローバルなオブジェクトをロック無しで触れている。
/// そのため [Component] はプロセス全体で共有のロックを保持し、[Component::init()] および [Component::re_init()] で
/// グローバルなロックをすることでスレッド安全性を確保している。
pub struct Component {
    ptr: *mut sys::AMFComponent,
    f: ComponentFuncs,
    /// コンポーネント固有の状態
    state: Arc<AmfObjectState>,
    /// プロセス全体で共有する状態
    process_state: Arc<AmfObjectState>,
    context: Context,
}

unsafe impl Send for Component {}
unsafe impl Sync for Component {}

impl Component {
    /// ハンドルを作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFComponent` を指すこと。
    unsafe fn from_raw_with_acquire(
        ptr: *mut sys::AMFComponent,
        acquire: bool,
        process_state: Arc<AmfObjectState>,
        context: Context,
    ) -> Result<Self, Error> {
        let state = AmfObjectState::new();
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
            get_caps: require_vtbl_fn(vtbl.GetCaps, "AMFComponent::GetCaps")?,
        };
        if acquire {
            unsafe { (f.acquire)(ptr) };
        }
        state.inc();
        process_state.inc();
        Ok(Self {
            ptr,
            f,
            state,
            process_state,
            context,
        })
    }

    /// プロセス全体の状態と Context を指定して生ポインタから Component を作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFComponent` を指すこと。`context` は `ptr` のコンポーネントを
    /// 作成した `Context` であること。
    pub(crate) unsafe fn from_raw_with_state(
        ptr: *mut sys::AMFComponent,
        process_state: Arc<AmfObjectState>,
        context: Context,
    ) -> Result<Self, Error> {
        unsafe { Self::from_raw_with_acquire(ptr, false, process_state, context) }
    }

    /// このコンポーネントが保持する `Context` を返す
    pub fn context(&self) -> Context {
        self.context.clone()
    }

    /// 生ポインタを返す。
    pub fn as_ptr(&self) -> *mut sys::AMFComponent {
        self.ptr
    }

    /// 所有権を放棄して生ポインタを返す (Release は呼ばない)。
    pub fn into_raw(mut self) -> *mut sys::AMFComponent {
        let ptr = self.ptr;
        self.ptr = std::ptr::null_mut();
        ptr
    }

    /// コンポーネントを初期化する
    pub fn init(&self, format: SurfaceFormat, width: i32, height: i32) -> sys::AMF_RESULT {
        let _guard = self.process_state.lock();
        unsafe { (self.f.init)(self.ptr, format.to_amf(), width, height) }
    }

    /// コンポーネントを再初期化する (解像度変更)
    pub fn re_init(&self, width: i32, height: i32) -> sys::AMF_RESULT {
        let _guard = self.process_state.lock();
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
    /// `data` は有効な AMFData を指すこと。`data` を指す他のハンドルが存在しないこと、
    /// およびコンポーネントが解放するまでアクセスしないことを呼び出し元が保証すること。
    pub unsafe fn submit_input(&self, data: *mut sys::AMFData) -> sys::AMF_RESULT {
        unsafe { (self.f.submit_input)(self.ptr, data) }
    }

    /// 出力データを問い合わせる
    ///
    /// # Safety
    /// `pp_data` は有効な `*mut *mut AMFData` を指すこと。`*pp_data` が非 null に
    /// なった場合、AMF の参照カウントモデルに従い、呼び出し元がその参照を
    /// `Release` すること。
    pub unsafe fn query_output(&self, pp_data: *mut *mut sys::AMFData) -> sys::AMF_RESULT {
        unsafe { (self.f.query_output)(self.ptr, pp_data) }
    }

    /// コンテキストを取得する (参照カウントは増えない)
    pub fn get_context(&self) -> *mut sys::AMFContext {
        unsafe { (self.f.get_context)(self.ptr) }
    }

    /// AMFPropertyStorage としてアクセスする
    pub fn property_storage(&self) -> Result<PropertyStorage, Error> {
        unsafe {
            PropertyStorage::from_raw_with_acquire(
                self.ptr as *mut sys::AMFPropertyStorage,
                true,
                Arc::clone(&self.state),
                self.context.clone(),
            )
        }
    }

    /// コンポーネントの能力情報を取得する
    ///
    /// `GetCaps` を利用できない場合はエラーを返す。
    pub(crate) fn get_caps(&self) -> Result<Caps, Error> {
        let _guard = self.process_state.lock();
        let mut caps: *mut sys::AMFCaps = std::ptr::null_mut();
        let result = unsafe { (self.f.get_caps)(self.ptr, &mut caps) };
        Error::check(result, "AMFComponent::GetCaps")?;
        if caps.is_null() {
            return Err(Error::new_custom(
                "AMFComponent::GetCaps",
                "returned null caps",
            ));
        }
        unsafe { Caps::from_raw(caps) }
    }
}

impl Clone for Component {
    fn clone(&self) -> Self {
        unsafe {
            (self.f.acquire)(self.ptr);
        }
        self.state.inc();
        self.process_state.inc();
        Self {
            ptr: self.ptr,
            f: self.f,
            state: Arc::clone(&self.state),
            process_state: Arc::clone(&self.process_state),
            context: self.context.clone(),
        }
    }
}

impl Drop for Component {
    fn drop(&mut self) {
        self.state.dec();
        self.process_state.dec();
        if self.ptr.is_null() {
            return;
        }
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

// ---------------------------------------------------------------------------
// AMFCaps / AMFIOCaps のラッパー
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn free(ptr: *mut std::ffi::c_void);
}

#[derive(Clone, Copy)]
struct CapsFuncs {
    release: unsafe extern "C" fn(*mut sys::AMFCaps) -> sys::amf_long,
    get_acceleration_type: unsafe extern "C" fn(*mut sys::AMFCaps) -> sys::AMF_ACCELERATION_TYPE,
    get_input_caps:
        unsafe extern "C" fn(*mut sys::AMFCaps, *mut *mut sys::AMFIOCaps) -> sys::AMF_RESULT,
    get_output_caps:
        unsafe extern "C" fn(*mut sys::AMFCaps, *mut *mut sys::AMFIOCaps) -> sys::AMF_RESULT,
    get_property_count: unsafe extern "C" fn(*mut sys::AMFCaps) -> sys::amf_size,
    get_property_at: unsafe extern "C" fn(
        *mut sys::AMFCaps,
        sys::amf_size,
        *mut sys::wchar_t,
        sys::amf_size,
        *mut sys::AMFVariantStruct,
    ) -> sys::AMF_RESULT,
    get_property: unsafe extern "C" fn(
        *mut sys::AMFCaps,
        *const sys::wchar_t,
        *mut sys::AMFVariantStruct,
    ) -> sys::AMF_RESULT,
}

/// `AMFCaps` の RAII ラッパー
pub(crate) struct Caps {
    ptr: *mut sys::AMFCaps,
    f: CapsFuncs,
}

impl Caps {
    /// 生ポインタから `Caps` を作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFCaps` を指し、呼び出し元が参照を所有していること。
    unsafe fn from_raw(ptr: *mut sys::AMFCaps) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let release = require_vtbl_fn(vtbl.Release, "AMFCaps::Release")?;
        let funcs = (|| {
            Ok(CapsFuncs {
                release,
                get_acceleration_type: require_vtbl_fn(
                    vtbl.GetAccelerationType,
                    "AMFCaps::GetAccelerationType",
                )?,
                get_input_caps: require_vtbl_fn(vtbl.GetInputCaps, "AMFCaps::GetInputCaps")?,
                get_output_caps: require_vtbl_fn(vtbl.GetOutputCaps, "AMFCaps::GetOutputCaps")?,
                get_property_count: require_vtbl_fn(
                    vtbl.GetPropertyCount,
                    "AMFCaps::GetPropertyCount",
                )?,
                get_property_at: require_vtbl_fn(vtbl.GetPropertyAt, "AMFCaps::GetPropertyAt")?,
                get_property: require_vtbl_fn(vtbl.GetProperty, "AMFCaps::GetProperty")?,
            })
        })();
        match funcs {
            Ok(f) => Ok(Self { ptr, f }),
            Err(e) => {
                unsafe { release(ptr) };
                Err(e)
            }
        }
    }

    /// AMF が報告するアクセラレーション種別を返す
    pub(crate) fn get_acceleration_type(&self) -> sys::AMF_ACCELERATION_TYPE {
        unsafe { (self.f.get_acceleration_type)(self.ptr) }
    }

    /// 入力の能力情報を返す
    pub(crate) fn get_input_caps(&self) -> Result<IoCaps, Error> {
        let mut io_caps: *mut sys::AMFIOCaps = std::ptr::null_mut();
        let result = unsafe { (self.f.get_input_caps)(self.ptr, &mut io_caps) };
        Error::check(result, "AMFCaps::GetInputCaps")?;
        if io_caps.is_null() {
            return Err(Error::new_custom(
                "AMFCaps::GetInputCaps",
                "returned null io caps",
            ));
        }
        unsafe { IoCaps::from_raw(io_caps) }
    }

    /// 出力の能力情報を返す
    pub(crate) fn get_output_caps(&self) -> Result<IoCaps, Error> {
        let mut io_caps: *mut sys::AMFIOCaps = std::ptr::null_mut();
        let result = unsafe { (self.f.get_output_caps)(self.ptr, &mut io_caps) };
        Error::check(result, "AMFCaps::GetOutputCaps")?;
        if io_caps.is_null() {
            return Err(Error::new_custom(
                "AMFCaps::GetOutputCaps",
                "returned null io caps",
            ));
        }
        unsafe { IoCaps::from_raw(io_caps) }
    }

    /// プロパティ数を返す
    pub(crate) fn get_property_count(&self) -> sys::amf_size {
        unsafe { (self.f.get_property_count)(self.ptr) }
    }

    /// インデックスでプロパティを取得する
    ///
    /// プロパティ名が 256 文字以上の場合はエラーを返す。
    /// 返した `Variant` は AMF がコピーした値であり、`Drop` で解放される。
    pub(crate) fn get_property_at(&self, index: sys::amf_size) -> Result<(String, Variant), Error> {
        const MAX_NAME_LEN: usize = 255;
        let mut name = vec![0 as sys::wchar_t; MAX_NAME_LEN + 1];
        let mut value = Variant::empty();
        let result = unsafe {
            (self.f.get_property_at)(
                self.ptr,
                index,
                name.as_mut_ptr(),
                name.len(),
                value.as_ptr(),
            )
        };
        Error::check(result, "AMFCaps::GetPropertyAt")?;
        let name = sys::from_wstring(&name);
        if name.chars().count() >= MAX_NAME_LEN {
            return Err(Error::new_custom(
                "AMFCaps::GetPropertyAt",
                "property name is too long",
            ));
        }
        Ok((name, value))
    }

    /// 名前でプロパティを取得する
    ///
    /// プロパティが存在しない場合は `None` を返す。
    pub(crate) fn get_property(&self, name: &str) -> Option<Variant> {
        let mut value = Variant::empty();
        let name_w = sys::to_wstring(name);
        let result = unsafe { (self.f.get_property)(self.ptr, name_w.as_ptr(), value.as_ptr()) };
        if result != sys::AMF_RESULT::AMF_OK {
            return None;
        }
        Some(value)
    }
}

impl Drop for Caps {
    fn drop(&mut self) {
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

/// `AMFVariantStruct` の RAII ラッパー
pub(crate) struct Variant {
    value: sys::AMFVariantStruct,
}

impl Variant {
    /// 空の variant を作成する
    fn empty() -> Self {
        Self {
            value: sys::AMFVariantStruct::empty(),
        }
    }

    /// 書き込み用のポインタを返す
    fn as_ptr(&mut self) -> *mut sys::AMFVariantStruct {
        &mut self.value
    }

    /// 読み取り用の参照を返す
    pub(crate) fn as_ref(&self) -> &sys::AMFVariantStruct {
        &self.value
    }

    /// Int64 の値を返す
    ///
    /// variant の型が Int64 でない場合は `None` を返す。
    pub(crate) fn as_int64(&self) -> Option<sys::amf_int64> {
        if self.value.type_ != sys::AMF_VARIANT_TYPE::AMF_VARIANT_INT64 {
            return None;
        }
        Some(unsafe { self.value.__bindgen_anon_1.int64Value })
    }

    /// Bool の値を返す
    ///
    /// variant の型が Bool でない場合は `None` を返す。
    pub(crate) fn as_bool(&self) -> Option<bool> {
        if self.value.type_ != sys::AMF_VARIANT_TYPE::AMF_VARIANT_BOOL {
            return None;
        }
        Some(unsafe { self.value.__bindgen_anon_1.boolValue != 0 })
    }

    /// Size の値を幅と高さの組で返す
    ///
    /// variant の型が Size でない場合は `None` を返す。
    pub(crate) fn as_size(&self) -> Option<(sys::amf_int32, sys::amf_int32)> {
        if self.value.type_ != sys::AMF_VARIANT_TYPE::AMF_VARIANT_SIZE {
            return None;
        }
        let size = unsafe { self.value.__bindgen_anon_1.sizeValue };
        Some((size.width, size.height))
    }
}

impl Drop for Variant {
    fn drop(&mut self) {
        unsafe {
            release_variant(&mut self.value);
        }
    }
}

/// `AMFVariantStruct` が保持する AMF 側の参照を解放する
unsafe fn release_variant(value: &mut sys::AMFVariantStruct) {
    // AMF は variant の文字列を libc の malloc で確保するため free で解放する。
    // インターフェースは AMFInterface::Release で参照カウントを減らす。
    if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_STRING {
        let ptr = unsafe { value.__bindgen_anon_1.stringValue };
        if !ptr.is_null() {
            unsafe { free(ptr.cast()) };
        }
    } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_WSTRING {
        let ptr = unsafe { value.__bindgen_anon_1.wstringValue };
        if !ptr.is_null() {
            unsafe { free(ptr.cast()) };
        }
    } else if value.type_ == sys::AMF_VARIANT_TYPE::AMF_VARIANT_INTERFACE {
        let interface = unsafe { value.__bindgen_anon_1.pInterface };
        if !interface.is_null() {
            let vtbl = unsafe { &*(*interface).pVtbl };
            if let Some(release) = vtbl.Release {
                unsafe { release(interface) };
            }
        }
    }
    value.type_ = sys::AMF_VARIANT_TYPE::AMF_VARIANT_EMPTY;
}

#[derive(Clone, Copy)]
struct IoCapsFuncs {
    release: unsafe extern "C" fn(*mut sys::AMFIOCaps) -> sys::amf_long,
    get_width_range:
        unsafe extern "C" fn(*mut sys::AMFIOCaps, *mut sys::amf_int32, *mut sys::amf_int32),
    get_height_range:
        unsafe extern "C" fn(*mut sys::AMFIOCaps, *mut sys::amf_int32, *mut sys::amf_int32),
    get_vert_align: unsafe extern "C" fn(*mut sys::AMFIOCaps) -> sys::amf_int32,
    get_num_of_formats: unsafe extern "C" fn(*mut sys::AMFIOCaps) -> sys::amf_int32,
    get_format_at: unsafe extern "C" fn(
        *mut sys::AMFIOCaps,
        sys::amf_int32,
        *mut sys::AMF_SURFACE_FORMAT,
        *mut sys::amf_bool,
    ) -> sys::AMF_RESULT,
    get_num_of_memory_types: unsafe extern "C" fn(*mut sys::AMFIOCaps) -> sys::amf_int32,
    get_memory_type_at: unsafe extern "C" fn(
        *mut sys::AMFIOCaps,
        sys::amf_int32,
        *mut sys::AMF_MEMORY_TYPE,
        *mut sys::amf_bool,
    ) -> sys::AMF_RESULT,
    is_interlaced_supported: unsafe extern "C" fn(*mut sys::AMFIOCaps) -> sys::amf_bool,
}

/// `AMFIOCaps` の RAII ラッパー
pub(crate) struct IoCaps {
    ptr: *mut sys::AMFIOCaps,
    f: IoCapsFuncs,
}

impl IoCaps {
    /// 生ポインタから `IoCaps` を作成する
    ///
    /// # Safety
    /// `ptr` は有効な `AMFIOCaps` を指し、呼び出し元が参照を所有していること。
    unsafe fn from_raw(ptr: *mut sys::AMFIOCaps) -> Result<Self, Error> {
        let vtbl = unsafe { &*(*ptr).pVtbl };
        let release = require_vtbl_fn(vtbl.Release, "AMFIOCaps::Release")?;
        let funcs = (|| {
            Ok(IoCapsFuncs {
                release,
                get_width_range: require_vtbl_fn(vtbl.GetWidthRange, "AMFIOCaps::GetWidthRange")?,
                get_height_range: require_vtbl_fn(
                    vtbl.GetHeightRange,
                    "AMFIOCaps::GetHeightRange",
                )?,
                get_vert_align: require_vtbl_fn(vtbl.GetVertAlign, "AMFIOCaps::GetVertAlign")?,
                get_num_of_formats: require_vtbl_fn(
                    vtbl.GetNumOfFormats,
                    "AMFIOCaps::GetNumOfFormats",
                )?,
                get_format_at: require_vtbl_fn(vtbl.GetFormatAt, "AMFIOCaps::GetFormatAt")?,
                get_num_of_memory_types: require_vtbl_fn(
                    vtbl.GetNumOfMemoryTypes,
                    "AMFIOCaps::GetNumOfMemoryTypes",
                )?,
                get_memory_type_at: require_vtbl_fn(
                    vtbl.GetMemoryTypeAt,
                    "AMFIOCaps::GetMemoryTypeAt",
                )?,
                is_interlaced_supported: require_vtbl_fn(
                    vtbl.IsInterlacedSupported,
                    "AMFIOCaps::IsInterlacedSupported",
                )?,
            })
        })();
        match funcs {
            Ok(f) => Ok(Self { ptr, f }),
            Err(e) => {
                unsafe { release(ptr) };
                Err(e)
            }
        }
    }

    /// 幅の範囲を返す
    pub(crate) fn width_range(&self) -> (sys::amf_int32, sys::amf_int32) {
        let mut min_width = 0;
        let mut max_width = 0;
        unsafe {
            (self.f.get_width_range)(self.ptr, &mut min_width, &mut max_width);
        }
        (min_width, max_width)
    }

    /// 高さの範囲を返す
    pub(crate) fn height_range(&self) -> (sys::amf_int32, sys::amf_int32) {
        let mut min_height = 0;
        let mut max_height = 0;
        unsafe {
            (self.f.get_height_range)(self.ptr, &mut min_height, &mut max_height);
        }
        (min_height, max_height)
    }

    /// 垂直アライメントを返す
    pub(crate) fn vertical_alignment(&self) -> sys::amf_int32 {
        unsafe { (self.f.get_vert_align)(self.ptr) }
    }

    /// 対応する surface format の一覧を返す
    pub(crate) fn formats(&self) -> Result<Vec<(sys::AMF_SURFACE_FORMAT, bool)>, Error> {
        let count = unsafe { (self.f.get_num_of_formats)(self.ptr) };
        let mut formats = Vec::new();
        for index in 0..count {
            let mut format = sys::AMF_SURFACE_FORMAT::AMF_SURFACE_UNKNOWN;
            let mut native = 0;
            let result =
                unsafe { (self.f.get_format_at)(self.ptr, index, &mut format, &mut native) };
            Error::check(result, "AMFIOCaps::GetFormatAt")?;
            formats.push((format, native != 0));
        }
        Ok(formats)
    }

    /// 対応する memory type の一覧を返す
    pub(crate) fn memory_types(&self) -> Result<Vec<(sys::AMF_MEMORY_TYPE, bool)>, Error> {
        let count = unsafe { (self.f.get_num_of_memory_types)(self.ptr) };
        let mut memory_types = Vec::new();
        for index in 0..count {
            let mut memory_type = sys::AMF_MEMORY_TYPE::AMF_MEMORY_UNKNOWN;
            let mut native = 0;
            let result = unsafe {
                (self.f.get_memory_type_at)(self.ptr, index, &mut memory_type, &mut native)
            };
            Error::check(result, "AMFIOCaps::GetMemoryTypeAt")?;
            memory_types.push((memory_type, native != 0));
        }
        Ok(memory_types)
    }

    /// インターレースに対応しているかを返す
    pub(crate) fn interlaced(&self) -> bool {
        unsafe { (self.f.is_interlaced_supported)(self.ptr) != 0 }
    }
}

impl Drop for IoCaps {
    fn drop(&mut self) {
        unsafe {
            (self.f.release)(self.ptr);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::AmfLibrary;
    use crate::types::FrameFormat;

    /// テスト用の Context を作成する
    fn create_context() -> Context {
        AmfLibrary::instance()
            .create_context()
            .expect("Context の作成に失敗")
    }

    /// テスト用の Surface を作成する
    fn create_surface(context: &Context) -> Surface {
        context
            .alloc_surface(
                MemoryType::Host,
                SurfaceFormat::from(FrameFormat::Nv12),
                320,
                240,
            )
            .expect("Surface の確保に失敗")
    }

    /// テスト用の Buffer を作成する
    fn create_buffer(context: &Context) -> Buffer {
        context
            .alloc_buffer(MemoryType::Host, 4096)
            .expect("Buffer の確保に失敗")
    }

    /// Surface と Buffer が確保元の Context を保持することを確認する
    #[test]
    fn test_handle_context_matches_source_context() {
        let context_a = create_context();
        let context_b = create_context();
        let surface_a = create_surface(&context_a);
        let surface_b = create_surface(&context_b);
        let buffer_a = create_buffer(&context_a);

        assert_eq!(
            surface_a.context(),
            context_a,
            "Surface が確保元の Context を保持していない"
        );
        assert_ne!(
            surface_a.context(),
            context_b,
            "別の Context の Surface が同じ Context と判定された"
        );
        assert_eq!(
            surface_b.context(),
            context_b,
            "Surface が確保元の Context を保持していない"
        );
        assert_eq!(
            buffer_a.context(),
            context_a,
            "Buffer が確保元の Context を保持していない"
        );
        assert_ne!(
            buffer_a.context(),
            context_b,
            "別の Context の Buffer が同じ Context と判定された"
        );

        // from_raw で再構築しても、指定した Context を保持する
        let raw_surface =
            unsafe { Surface::from_raw(surface_a.clone().into_raw(), context_a.clone()) }
                .expect("Surface の再構築に失敗");
        assert_eq!(
            raw_surface.context(),
            context_a,
            "from_raw で再構築した Surface が Context を保持していない"
        );

        let cloned_context = context_a.clone();
        assert_eq!(
            cloned_context, context_a,
            "Clone した Context が元の Context と等しくない"
        );
        assert_ne!(
            context_a, context_b,
            "別々に作成した Context が等しいと判定された"
        );
    }

    /// Plane が Surface と同じ状態を共有することを確認する
    #[test]
    fn test_plane_shares_state_with_surface() {
        let context = create_context();
        let surface = create_surface(&context);

        assert!(
            !surface.is_shared(),
            "Plane を取得する前の Surface が共有扱いになっている"
        );
        let plane = surface
            .get_plane(PlaneType::Y)
            .expect("Y プレーンの取得に失敗");
        assert!(
            surface.is_shared(),
            "Plane を取得した Surface が共有扱いになっていない"
        );

        let cloned = plane.clone();
        drop(cloned);
        assert!(
            surface.is_shared(),
            "Plane の Clone を drop しても、Plane が生存している Surface が共有扱いになっていない"
        );

        drop(plane);
        assert!(
            !surface.is_shared(),
            "Plane を drop した後の Surface が共有扱いになっている"
        );
    }

    /// PropertyStorage が Surface と同じ状態を共有することを確認する
    #[test]
    fn test_property_storage_shares_state_with_surface() {
        let context = create_context();
        let surface = create_surface(&context);
        let property_storage = surface
            .property_storage()
            .expect("PropertyStorage の取得に失敗");

        assert!(
            Arc::ptr_eq(&surface.state, &property_storage.state),
            "PropertyStorage が Surface の状態を共有していない"
        );
    }

    /// Component の PropertyStorage が Component と同じ状態を共有することを確認する
    #[test]
    fn test_component_property_storage_shares_state_with_component() {
        let context = create_context();
        let component = AmfLibrary::instance()
            .create_component(&context, sys::str::AMFVideoEncoderVCE_AVC)
            .expect("Component の作成に失敗");
        let property_storage = component
            .property_storage()
            .expect("PropertyStorage の取得に失敗");

        assert!(
            Arc::ptr_eq(&component.state, &property_storage.state),
            "PropertyStorage が Component の状態を共有していない"
        );
        assert_eq!(
            component.context(),
            property_storage.context(),
            "PropertyStorage の Context が Component の Context と一致しない"
        );
    }

    /// from_raw_acquired で作成したハンドルが状態を共有しないことを確認する
    #[test]
    fn test_from_raw_acquired_does_not_share_state() {
        let context = create_context();
        let surface = create_surface(&context);
        let clone = surface.clone();
        let other = unsafe { Surface::from_raw_acquired(surface.as_ptr(), context.clone()) }
            .expect("Surface の再構築に失敗");

        assert!(
            !Arc::ptr_eq(&surface.state, &other.state),
            "from_raw_acquired が元のハンドルと状態を共有している"
        );
        assert!(
            !Arc::ptr_eq(&clone.state, &other.state),
            "from_raw_acquired が Clone と状態を共有している"
        );

        // 元のハンドルと Clone は引き続き利用できる
        let plane = surface
            .get_plane(PlaneType::Y)
            .expect("Y プレーンの取得に失敗");
        assert_eq!(plane.get_width(), 320, "幅が一致しない");
        drop(plane);
        let plane = clone
            .get_plane(PlaneType::Y)
            .expect("Y プレーンの取得に失敗");
        assert_eq!(plane.get_width(), 320, "幅が一致しない");
        drop(plane);

        // 元のハンドルと Clone を drop しても、Acquire 済みの other は利用できる
        drop(clone);
        drop(surface);
        let plane = other
            .get_plane(PlaneType::Y)
            .expect("Y プレーンの取得に失敗");
        assert_eq!(plane.get_width(), 320, "幅が一致しない");
    }

    /// Surface の is_shared がハンドル数に応じて変化することを確認する
    #[test]
    fn test_surface_is_shared_tracks_handles() {
        let context = create_context();
        let surface = create_surface(&context);

        assert!(
            !surface.is_shared(),
            "作成直後の Surface が共有扱いになっている"
        );
        let clone = surface.clone();
        assert!(
            surface.is_shared(),
            "Clone 後の Surface が共有扱いになっていない"
        );
        assert!(
            clone.is_shared(),
            "Clone 側の Surface が共有扱いになっていない"
        );
        drop(clone);
        assert!(
            !surface.is_shared(),
            "Clone を drop した後の Surface が共有扱いになっている"
        );
    }

    /// Buffer の is_shared がハンドル数に応じて変化することを確認する
    #[test]
    fn test_buffer_is_shared_tracks_handles() {
        let context = create_context();
        let buffer = create_buffer(&context);

        assert!(
            !buffer.is_shared(),
            "作成直後の Buffer が共有扱いになっている"
        );
        let clone = buffer.clone();
        assert!(
            buffer.is_shared(),
            "Clone 後の Buffer が共有扱いになっていない"
        );
        assert!(
            clone.is_shared(),
            "Clone 側の Buffer が共有扱いになっていない"
        );
        drop(clone);
        assert!(
            !buffer.is_shared(),
            "Clone を drop した後の Buffer が共有扱いになっている"
        );
    }

    /// Surface を複数スレッドから参照で並行利用できることを確認する
    #[test]
    fn test_surface_can_be_used_from_multiple_threads_by_reference() {
        let context = create_context();
        let surface = create_surface(&context);

        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for i in 0..100 {
                        surface.set_pts(i);
                        let plane = surface
                            .get_plane(PlaneType::Y)
                            .expect("Y プレーンの取得に失敗");
                        assert!(
                            unsafe { plane.as_slice() }.is_ok(),
                            "Y プレーンをスライスとして取得できない"
                        );
                    }
                });
            }
        });
    }

    /// null ポインタがスライス生成の対象にならないことを確認する
    #[test]
    fn test_validate_slice_ptr_rejects_null() {
        assert!(
            validate_slice_ptr("test", std::ptr::null()).is_err(),
            "null ポインタが受け入れられた"
        );

        let data = [0u8; 1];
        assert!(
            validate_slice_ptr("test", data.as_ptr()).is_ok(),
            "null でないポインタが拒否された"
        );
    }

    /// isize::MAX を超える長さがスライスの長さとして拒否されることを確認する
    #[test]
    fn test_validate_slice_len_rejects_too_long() {
        assert!(
            validate_slice_len("test", isize::MAX as usize).is_ok(),
            "isize::MAX の長さが拒否された"
        );
        assert!(
            validate_slice_len("test", isize::MAX as usize + 1).is_err(),
            "isize::MAX を超える長さが受け入れられた"
        );
    }

    /// 負のピッチと積のオーバーフローが拒否されることを確認する
    #[test]
    fn test_plane_slice_len_validates_pitch() {
        assert_eq!(
            plane_slice_len("test", false, 4, 3).expect("正のピッチが拒否された"),
            12,
            "ピッチの積が一致しない"
        );
        assert!(
            plane_slice_len("test", false, -1, 3).is_err(),
            "負の hpitch が受け入れられた"
        );
        assert!(
            plane_slice_len("test", false, 4, -1).is_err(),
            "負の vpitch が受け入れられた"
        );
        assert!(
            checked_mul_len("test", usize::MAX, 2).is_err(),
            "長さの積のオーバーフローが受け入れられた"
        );
        assert!(
            plane_slice_len("test", true, 4, 3).is_err(),
            "タイル状のプレーンが受け入れられた"
        );
    }
}
