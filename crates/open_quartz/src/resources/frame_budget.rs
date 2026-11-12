//! Shared physical frame allocations, independent of codec and GPU APIs.
use std::collections::HashMap;
use std::fmt;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex, MutexGuard, TryLockError, Weak,
};

type AllocationKey = (u8, usize);
type Waker = Arc<dyn Fn() + Send + Sync>;
type Wakers = Arc<Vec<(usize, Waker)>>;

fn wake(legacy: Option<Waker>, registered: Wakers) {
    if let Some(waker) = legacy {
        waker();
    }
    for (_, waker) in registered.iter() {
        waker();
    }
}

struct Allocation {
    references: usize,
    surfaces: usize,
    bytes: u64,
}

struct State {
    allocations: HashMap<AllocationKey, Allocation>,
    surfaces: usize,
    bytes: u64,
    waker: Option<Waker>,
    registered_wakers: Wakers,
    next_waker: usize,
}

/// One coordinator for native surfaces (domain 0), stable textures (domain 1),
/// and reserved working sets (domain 2). Identical physical identities charge once.
/// Callers must keep a credit alive for the allocation's entire physical lifetime.
pub struct FrameBudget {
    max_surfaces: usize,
    max_bytes: u64,
    state: Mutex<State>,
    admission: Mutex<()>,
    admission_waiting: AtomicBool,
}

/// Registration lifetime follows the session, not the shared device.
pub struct FrameWakeRegistration {
    budget: Weak<FrameBudget>,
    id: usize,
}

pub struct FrameAdmissionGuard<'a> {
    budget: &'a FrameBudget,
    guard: Option<MutexGuard<'a, ()>>,
}

impl Drop for FrameAdmissionGuard<'_> {
    fn drop(&mut self) {
        drop(self.guard.take());
        if self.budget.admission_waiting.swap(false, Ordering::AcqRel) {
            self.budget.notify_waiters();
        }
    }
}

impl Drop for FrameWakeRegistration {
    fn drop(&mut self) {
        if let Some(budget) = self.budget.upgrade() {
            let mut state = budget
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            Arc::make_mut(&mut state.registered_wakers).retain(|(id, _)| *id != self.id);
        }
    }
}

impl FrameBudget {
    pub fn new(max_surfaces: usize, max_bytes: u64) -> Result<Arc<Self>, String> {
        if max_surfaces == 0 || max_bytes == 0 {
            return Err("frame budget limits must be nonzero".into());
        }
        Ok(Arc::new(Self {
            max_surfaces,
            max_bytes,
            admission: Mutex::new(()),
            admission_waiting: AtomicBool::new(false),
            state: Mutex::new(State {
                allocations: HashMap::new(),
                surfaces: 0,
                bytes: 0,
                waker: None,
                registered_wakers: Arc::new(Vec::new()),
                next_waker: 0,
            }),
        }))
    }

    /// Serialize downstream reservations and source admission, never GPU/codec
    /// completion. Contention is temporary and wakes the sharing sessions.
    pub fn try_admission(&self) -> Result<Option<FrameAdmissionGuard<'_>>, String> {
        match self.admission.try_lock() {
            Ok(guard) => Ok(Some(FrameAdmissionGuard {
                budget: self,
                guard: Some(guard),
            })),
            Err(TryLockError::WouldBlock) => {
                self.admission_waiting.store(true, Ordering::Release);
                // Close the unlock/registration race without waiting.
                match self.admission.try_lock() {
                    Ok(guard) => Ok(Some(FrameAdmissionGuard {
                        budget: self,
                        guard: Some(guard),
                    })),
                    Err(TryLockError::WouldBlock) => Ok(None),
                    Err(TryLockError::Poisoned(_)) => Err("frame admission lock poisoned".into()),
                }
            }
            Err(TryLockError::Poisoned(_)) => Err("frame admission lock poisoned".into()),
        }
    }

    pub fn limits(&self) -> (usize, u64) {
        (self.max_surfaces, self.max_bytes)
    }

    pub fn set_waker(&self, waker: Waker) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .waker = Some(waker);
    }

    pub fn register_waker(self: &Arc<Self>, waker: Waker) -> FrameWakeRegistration {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let id = state.next_waker;
        state.next_waker = id
            .checked_add(1)
            .expect("frame wake registration identity exhausted");
        Arc::make_mut(&mut state.registered_wakers).push((id, waker));
        FrameWakeRegistration {
            budget: Arc::downgrade(self),
            id,
        }
    }

    /// A stage-local slot can become available while its shared physical
    /// allocation remains retained by another stage. Wake every interested
    /// session even when no physical charge was released.
    pub(crate) fn notify_waiters(&self) {
        let (legacy, registered) = {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            (state.waker.clone(), Arc::clone(&state.registered_wakers))
        };
        wake(legacy, registered);
    }

    pub fn validate_working_set(&self, surfaces: usize, bytes: u64) -> Result<(), String> {
        if surfaces > self.max_surfaces || bytes > self.max_bytes {
            return Err("physical frame allocation exceeds shared frame budget".into());
        }
        Ok(())
    }

    fn matching(allocation: &Allocation, surfaces: usize, bytes: u64) -> Result<(), String> {
        if allocation.surfaces != surfaces || allocation.bytes != bytes {
            return Err("physical frame identity has inconsistent allocation size".into());
        }
        Ok(())
    }

    pub fn can_acquire(
        &self,
        key: AllocationKey,
        surfaces: usize,
        bytes: u64,
    ) -> Result<bool, String> {
        self.validate_working_set(surfaces, bytes)?;
        let state = self
            .state
            .lock()
            .map_err(|_| "frame budget lock poisoned")?;
        if let Some(allocation) = state.allocations.get(&key) {
            Self::matching(allocation, surfaces, bytes)?;
            return Ok(true);
        }
        Ok(surfaces <= self.max_surfaces - state.surfaces && bytes <= self.max_bytes - state.bytes)
    }

    pub fn try_acquire(
        self: &Arc<Self>,
        key: AllocationKey,
        surfaces: usize,
        bytes: u64,
    ) -> Result<Option<FrameCredit>, String> {
        self.validate_working_set(surfaces, bytes)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| "frame budget lock poisoned")?;
        if let Some(allocation) = state.allocations.get_mut(&key) {
            Self::matching(allocation, surfaces, bytes)?;
            allocation.references = allocation
                .references
                .checked_add(1)
                .ok_or("frame reference count overflow")?;
        } else {
            if surfaces > self.max_surfaces - state.surfaces || bytes > self.max_bytes - state.bytes
            {
                return Ok(None);
            }
            state.allocations.insert(
                key,
                Allocation {
                    references: 1,
                    surfaces,
                    bytes,
                },
            );
            state.surfaces += surfaces;
            state.bytes += bytes;
        }
        Ok(Some(FrameCredit {
            budget: Arc::clone(self),
            key,
        }))
    }

    pub fn usage(&self) -> (usize, u64) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        (state.surfaces, state.bytes)
    }
}

/// A reference to a charged physical identity, not a copy of its charge.
pub struct FrameCredit {
    budget: Arc<FrameBudget>,
    key: AllocationKey,
}

impl fmt::Debug for FrameCredit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameCredit")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl Clone for FrameCredit {
    fn clone(&self) -> Self {
        let mut state = self
            .budget
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let allocation = state
            .allocations
            .get_mut(&self.key)
            .expect("live frame credit has an allocation");
        allocation.references = allocation
            .references
            .checked_add(1)
            .expect("frame reference count overflow");
        Self {
            budget: Arc::clone(&self.budget),
            key: self.key,
        }
    }
}

impl FrameCredit {
    /// Atomically converts this reference's reservation into an actual identity.
    /// False leaves the original credit unchanged when the replacement cannot fit.
    /// Other clones of the original reservation retain their original charge.
    pub fn reassign(
        &mut self,
        key: AllocationKey,
        surfaces: usize,
        bytes: u64,
    ) -> Result<bool, String> {
        self.budget.validate_working_set(surfaces, bytes)?;
        let waker = {
            let mut state = self
                .budget
                .state
                .lock()
                .map_err(|_| "frame budget lock poisoned")?;
            let old = state
                .allocations
                .get(&self.key)
                .expect("live frame credit has an allocation");
            if key == self.key {
                FrameBudget::matching(old, surfaces, bytes)?;
                return Ok(true);
            }
            let last_reference = old.references == 1;
            let (released_surfaces, released_bytes) = if last_reference {
                (old.surfaces, old.bytes)
            } else {
                (0, 0)
            };
            let (new_surfaces, new_bytes) = if let Some(target) = state.allocations.get(&key) {
                FrameBudget::matching(target, surfaces, bytes)?;
                if target.references == usize::MAX {
                    return Err("frame reference count overflow".into());
                }
                (0, 0)
            } else {
                (surfaces, bytes)
            };
            let remaining_surfaces = state.surfaces - released_surfaces;
            let remaining_bytes = state.bytes - released_bytes;
            if new_surfaces > self.budget.max_surfaces - remaining_surfaces
                || new_bytes > self.budget.max_bytes - remaining_bytes
            {
                return Ok(false);
            }
            if last_reference {
                state.allocations.remove(&self.key);
            } else {
                state.allocations.get_mut(&self.key).unwrap().references -= 1;
            }
            if let Some(target) = state.allocations.get_mut(&key) {
                target.references += 1;
            } else {
                state.allocations.insert(
                    key,
                    Allocation {
                        references: 1,
                        surfaces,
                        bytes,
                    },
                );
            }
            state.surfaces = remaining_surfaces + new_surfaces;
            state.bytes = remaining_bytes + new_bytes;
            self.key = key;
            if released_surfaces > new_surfaces || released_bytes > new_bytes {
                Some((state.waker.clone(), Arc::clone(&state.registered_wakers)))
            } else {
                None
            }
        };
        if let Some((legacy, registered)) = waker {
            wake(legacy, registered);
        }
        Ok(true)
    }
}

impl Drop for FrameCredit {
    fn drop(&mut self) {
        let waker = {
            let mut state = self
                .budget
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let allocation = state
                .allocations
                .get_mut(&self.key)
                .expect("live frame credit has an allocation");
            allocation.references -= 1;
            if allocation.references != 0 {
                return;
            }
            let allocation = state.allocations.remove(&self.key).unwrap();
            state.surfaces -= allocation.surfaces;
            state.bytes -= allocation.bytes;
            (state.waker.clone(), Arc::clone(&state.registered_wakers))
        };
        wake(waker.0, waker.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn shared_identities_clones_domains_and_last_drop() {
        let budget = FrameBudget::new(2, 100).unwrap();
        let first = budget.try_acquire((0, 42), 1, 40).unwrap().unwrap();
        let same = budget.try_acquire((0, 42), 1, 40).unwrap().unwrap();
        let clone = first.clone();
        assert_eq!(budget.usage(), (1, 40));
        let other = budget.try_acquire((1, 42), 1, 60).unwrap().unwrap();
        assert_eq!(budget.usage(), (2, 100));
        assert!(budget.try_acquire((2, 7), 1, 1).unwrap().is_none());
        drop((first, same));
        assert_eq!(budget.usage(), (2, 100));
        drop(clone);
        assert_eq!(budget.usage(), (1, 60));
        drop(other);
        assert_eq!(budget.usage(), (0, 0));
    }

    #[test]
    fn oversized_conflicting_and_byte_saturated_requests_are_distinct() {
        assert!(FrameBudget::new(0, 10).is_err());
        assert!(FrameBudget::new(1, 0).is_err());
        let budget = FrameBudget::new(3, 100).unwrap();
        assert!(budget.try_acquire((0, 1), 4, 1).is_err());
        assert!(budget.try_acquire((0, 1), 1, 101).is_err());
        let held = budget.try_acquire((0, 1), 1, 90).unwrap().unwrap();
        assert!(budget.try_acquire((0, 1), 1, 91).is_err());
        assert!(budget.try_acquire((0, 2), 1, 11).unwrap().is_none());
        assert!(!budget.can_acquire((0, 2), 1, 11).unwrap());
        drop(held);
        assert!(budget.can_acquire((0, 2), 1, 11).unwrap());
    }

    #[test]
    fn reservation_conversion_is_atomic_and_deduplicates_existing_surfaces() {
        let budget = FrameBudget::new(2, 100).unwrap();
        let native = budget.try_acquire((0, 3), 1, 60).unwrap().unwrap();
        let mut reserved = budget.try_acquire((2, 4), 1, 40).unwrap().unwrap();
        assert!(!reserved.reassign((0, 5), 1, 50).unwrap());
        assert_eq!(budget.usage(), (2, 100));
        assert!(reserved.reassign((0, 3), 1, 60).unwrap());
        assert_eq!(budget.usage(), (1, 60));
        drop(native);
        assert_eq!(budget.usage(), (1, 60));
        let mut second = budget.try_acquire((2, 6), 1, 40).unwrap().unwrap();
        assert!(second.reassign((0, 7), 1, 40).unwrap());
        assert_eq!(budget.usage(), (2, 100));
        drop((reserved, second));
        assert_eq!(budget.usage(), (0, 0));
    }

    #[test]
    fn release_wakes_outside_lock_and_shared_saturation_retries() {
        let budget = FrameBudget::new(1, 64).unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        let weak = Arc::downgrade(&budget);
        budget.set_waker(Arc::new(move || {
            assert_eq!(weak.upgrade().unwrap().usage(), (0, 0));
            count.fetch_add(1, Ordering::Relaxed);
        }));
        let held = budget.try_acquire((1, 1), 1, 64).unwrap().unwrap();
        assert!(budget.try_acquire((0, 2), 1, 1).unwrap().is_none());
        drop(held);
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        let retry = budget.try_acquire((0, 2), 1, 1).unwrap().unwrap();
        drop(retry);
        assert_eq!(wakes.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn registered_sessions_all_wake_and_disposed_sessions_unregister() {
        let budget = FrameBudget::new(1, 16).unwrap();
        let first = Arc::new(AtomicUsize::new(0));
        let second = Arc::new(AtomicUsize::new(0));
        let count = first.clone();
        let weak = Arc::downgrade(&budget);
        let registration = budget.register_waker(Arc::new(move || {
            assert_eq!(weak.upgrade().unwrap().usage(), (0, 0));
            count.fetch_add(1, Ordering::Relaxed);
        }));
        let count = second.clone();
        let other = budget.register_waker(Arc::new(move || {
            count.fetch_add(1, Ordering::Relaxed);
        }));
        drop(budget.try_acquire((0, 1), 1, 16).unwrap().unwrap());
        assert_eq!(first.load(Ordering::Relaxed), 1);
        assert_eq!(second.load(Ordering::Relaxed), 1);
        drop(registration);
        drop(budget.try_acquire((0, 1), 1, 16).unwrap().unwrap());
        assert_eq!(first.load(Ordering::Relaxed), 1);
        assert_eq!(second.load(Ordering::Relaxed), 2);
        drop(other);
        drop(budget.try_acquire((0, 1), 1, 16).unwrap().unwrap());
        assert_eq!(first.load(Ordering::Relaxed), 1);
        assert_eq!(second.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn concurrent_session_reservations_do_not_wait_or_interleave_and_wake_on_release() {
        let budget = FrameBudget::new(4, 64).unwrap();
        let admitted = budget.try_admission().unwrap().unwrap();
        let wakes = Arc::new(AtomicUsize::new(0));
        let count = wakes.clone();
        let weak = Arc::downgrade(&budget);
        let _registration = budget.register_waker(Arc::new(move || {
            let budget = weak.upgrade().unwrap();
            assert!(
                budget.try_admission().unwrap().is_some(),
                "wake ran before the reservation gate unlocked"
            );
            count.fetch_add(1, Ordering::Relaxed);
        }));
        let other = budget.clone();
        std::thread::spawn(move || {
            assert!(other.try_admission().unwrap().is_none());
        })
        .join()
        .unwrap();
        let downstream = budget.try_acquire((1, 1), 2, 32).unwrap().unwrap();
        let source = budget.try_acquire((2, 1), 2, 32).unwrap().unwrap();
        assert_eq!(wakes.load(Ordering::Relaxed), 0);
        drop(admitted);
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert!(budget.try_admission().unwrap().is_some());
        drop(_registration);
        drop((downstream, source));
        assert_eq!(budget.usage(), (0, 0));
    }
}
