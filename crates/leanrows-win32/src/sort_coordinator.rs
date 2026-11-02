//! One process-wide sorting reservation, shared by every document worker.
use leanrows_core::SortResources;
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};
#[derive(Clone)]
pub(crate) struct SortCoordinator(Arc<Inner>);
struct Inner {
    current: AtomicU64,
    active: AtomicU64,
    resources: Arc<SortResources>,
}
impl SortCoordinator {
    pub(crate) fn shared() -> Self {
        static SHARED: OnceLock<SortCoordinator> = OnceLock::new();
        SHARED.get_or_init(Self::new).clone()
    }
    pub(crate) fn new() -> Self {
        Self(Arc::new(Inner {
            current: AtomicU64::new(0),
            active: AtomicU64::new(0),
            resources: SortResources::new(16 * 1024 * 1024, 8 * 1024 * 1024 * 1024),
        }))
    }
    pub(crate) fn request(&self) -> u64 {
        self.0.current.fetch_add(1, Ordering::AcqRel) + 1
    }
    pub(crate) fn is_current(&self, serial: u64) -> bool {
        self.0.current.load(Ordering::Acquire) == serial
    }
    pub(crate) fn try_acquire(&self, serial: u64) -> Option<SortPermit> {
        if !self.is_current(serial) {
            return None;
        }
        self.0
            .active
            .compare_exchange(0, serial, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        let permit = SortPermit {
            coordinator: self.clone(),
            serial,
        };
        self.is_current(serial).then_some(permit)
    }
    pub(crate) fn resources(&self) -> Arc<SortResources> {
        self.0.resources.clone()
    }
}
pub(crate) struct SortPermit {
    coordinator: SortCoordinator,
    serial: u64,
}
impl Drop for SortPermit {
    fn drop(&mut self) {
        let _ = self.coordinator.0.active.compare_exchange(
            self.serial,
            0,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_tab_waits_until_old_allocation_is_released() {
        let coordinator = SortCoordinator::new();
        let first_serial = coordinator.request();
        let first = coordinator.try_acquire(first_serial);
        assert!(first.is_some());
        let next = coordinator.request();
        assert!(!coordinator.is_current(first_serial));
        assert!(coordinator.try_acquire(next).is_none());
        drop(first);
        assert!(coordinator.try_acquire(next).is_some());
    }
}
