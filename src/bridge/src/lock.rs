//! A small helper for locking std's Mutex.

use std::sync::{Mutex, MutexGuard};

/// Locks a Mutex. Updates inside never panic halfway, so a poisoned state is used as is
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
