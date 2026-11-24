//! Page-locked storage for long-lived key material (v0.6 plan H6).
//!
//! The VaultKey lives for the whole unlocked session. [`LockedKey`] keeps it in
//! its own 4 KiB page (a page-aligned, page-sized heap allocation, so no other
//! data shares the page and unlocking it never affects another allocation) and
//! locks that page in RAM (`VirtualLock` on Windows, `mlock` elsewhere, through
//! the `region` crate), so the key is not written to the pagefile. The page is
//! locked *before* the key is copied in, and wiped before it is unlocked and freed.
//!
//! Locking is best effort: if the OS refuses (working-set quota, a page size
//! other than 4 KiB), the key still works and is still wiped on drop;
//! [`LockedKey::is_locked`] reports the outcome. Short-lived keys (KEKs) stay
//! `Zeroizing` arrays: they exist for one wrap / unwrap call.

use std::fmt;

use zeroize::Zeroize;

use crate::crypto::KEY_LEN;

const PAGE_SIZE: usize = 4096;

/// One page, aligned to a page boundary: the global allocator gives it a page of
/// its own.
#[repr(C, align(4096))]
struct Page([u8; PAGE_SIZE]);

pub struct LockedKey {
    page: Box<Page>,
    /// Unlocks the page when dropped; `None` when locking failed or is not
    /// possible on this system.
    guard: Option<region::LockGuard>,
}

impl LockedKey {
    pub fn new(key: &[u8; KEY_LEN]) -> Self {
        let mut page = Box::new(Page([0; PAGE_SIZE]));
        let guard = lock_page(&page);
        page.0[..KEY_LEN].copy_from_slice(key);
        Self { page, guard }
    }

    /// Copies a key from a buffer of exactly [`KEY_LEN`] bytes.
    pub fn from_slice(key: &[u8]) -> Option<Self> {
        let key: &[u8; KEY_LEN] = key.try_into().ok()?;
        Some(Self::new(key))
    }

    /// True when the page holding the key is locked in RAM.
    pub fn is_locked(&self) -> bool {
        self.guard.is_some()
    }
}

fn lock_page(page: &Page) -> Option<region::LockGuard> {
    // With larger pages (e.g. 16 KiB) the allocation would share its page.
    if region::page::size() != PAGE_SIZE {
        return None;
    }
    region::lock(page.0.as_ptr(), PAGE_SIZE).ok()
}

impl std::ops::Deref for LockedKey {
    type Target = [u8; KEY_LEN];

    fn deref(&self) -> &Self::Target {
        self.page.0.first_chunk().expect("a page is larger than a key")
    }
}

impl Clone for LockedKey {
    /// A clone gets its own locked page.
    fn clone(&self) -> Self {
        Self::new(self)
    }
}

impl Drop for LockedKey {
    fn drop(&mut self) {
        // Wipe while the page is still locked, then unlock; the box frees it after.
        self.page.0.zeroize();
        self.guard.take();
    }
}

impl fmt::Debug for LockedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LockedKey")
            .field("locked", &self.is_locked())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_the_key_in_its_own_page() {
        let key = [7u8; KEY_LEN];
        let locked = LockedKey::new(&key);
        assert_eq!(*locked, key);
        let addr = locked.page.0.as_ptr() as usize;
        assert_eq!(addr % PAGE_SIZE, 0, "page-aligned");
    }

    #[cfg(windows)]
    #[test]
    fn page_is_locked_on_windows() {
        // A few pages fit in the default minimum working set.
        assert!(LockedKey::new(&[1; KEY_LEN]).is_locked());
    }

    #[test]
    fn clone_gets_a_separate_page_with_the_same_key() {
        let a = LockedKey::new(&[3; KEY_LEN]);
        let b = a.clone();
        assert_eq!(*a, *b);
        assert_ne!(a.page.0.as_ptr(), b.page.0.as_ptr());
        assert_eq!(a.is_locked(), b.is_locked());
        drop(a);
        assert_eq!(*b, [3; KEY_LEN], "dropping one copy leaves the other intact");
    }

    #[test]
    fn from_slice_checks_the_length() {
        assert!(LockedKey::from_slice(&[0; KEY_LEN - 1]).is_none());
        assert_eq!(*LockedKey::from_slice(&[9; KEY_LEN]).unwrap(), [9; KEY_LEN]);
    }

    #[test]
    fn debug_does_not_print_the_key() {
        let text = format!("{:?}", LockedKey::new(&[0x41; KEY_LEN]));
        assert!(!text.contains("65") && !text.contains("41"), "{text}");
    }
}
