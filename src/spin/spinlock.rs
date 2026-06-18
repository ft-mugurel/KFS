use crate::x86::{disable_interrupts, read_eflags, restore_eflags};
use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};

pub struct Spinlock<T> {
    is_locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for Spinlock<T> {}
unsafe impl<T: Send> Send for Spinlock<T> {}

impl<T> Spinlock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            is_locked: AtomicBool::new(false),
            data: UnsafeCell::new(value),
        }
    }

    #[inline(never)]
    pub fn lock(&self) -> SpinlockGuard<'_, T> {
        unsafe {
            let flags = read_eflags();
            disable_interrupts();

            while self
                .is_locked
                .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_err()
            {
                core::hint::spin_loop();
            }

            SpinlockGuard { lock: self, saved_flags: flags }
        }
    }
}

pub struct SpinlockGuard<'a, T> {
    lock: &'a Spinlock<T>,
    saved_flags: u32,
}

impl<T> Deref for SpinlockGuard<'_, T> {
    type Target = T;

    #[inline(always)]
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<T> DerefMut for SpinlockGuard<'_, T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T> Drop for SpinlockGuard<'_, T> {
    #[inline(always)]
    fn drop(&mut self) {
        self.lock.is_locked.store(false, Ordering::Release);
        unsafe {
            restore_eflags(self.saved_flags);
        }
    }
}
