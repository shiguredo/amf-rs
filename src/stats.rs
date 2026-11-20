//! 統計値取得用のプリミティブ型

use std::sync::atomic::{AtomicU64, Ordering};

/// 単調増加カウンター
///
/// 通算値を保持する。値は crate 内部で更新され、[`Counter::get`] で現在値を
/// 読み出せる。読み出しは原子的に行われるため、複数のスレッドから同じ値を
/// 読める。
#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    /// 0 で初期化したカウンターを生成する
    pub fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// 現在の値を読み出す
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    /// カウンターを 1 増やす
    pub(crate) fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// カウンターを指定した値だけ増やす
    pub(crate) fn add(&self, value: u64) {
        self.0.fetch_add(value, Ordering::Relaxed);
    }
}

impl Clone for Counter {
    /// 現在値のコピーを取得する
    ///
    /// 元のカウンターとは独立した値であり、以後の変更は互いに影響しない。
    fn clone(&self) -> Self {
        Self(AtomicU64::new(self.get()))
    }
}

/// 現在値を表すゲージ
///
/// 時点値を保持する。[`Counter`] が通算値なのに対し、こちらは増減によって
/// 現在の状態を表す。値は crate 内部で更新され、[`Gauge::get`] で現在値を
/// 読み出せる。読み出しは原子的に行われるため、複数のスレッドから同じ値を
/// 読める。
#[derive(Debug, Default)]
pub struct Gauge(AtomicU64);

impl Gauge {
    /// 0 で初期化したゲージを生成する
    pub fn new() -> Self {
        Self(AtomicU64::new(0))
    }

    /// 現在の値を読み出す
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }

    /// 値を 1 増やす
    pub(crate) fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }

    /// 値を 1 減らす
    ///
    /// 現在値が 0 の場合は 0 のまま変化しない。
    pub(crate) fn dec(&self) {
        // 0 を下回ると u64 の最大値に回り込むため、飽和減算する。
        let _ = self
            .0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                Some(current.saturating_sub(1))
            });
    }
}

impl Clone for Gauge {
    /// 現在値のコピーを取得する
    ///
    /// 元のゲージとは独立した値であり、以後の変更は互いに影響しない。
    fn clone(&self) -> Self {
        Self(AtomicU64::new(self.get()))
    }
}
