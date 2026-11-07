//! Shared immutable image ownership for legacy and budgeted image paths.
//!
//! Cloning a handle retains its existing allocation; it never copies pixels.
//! Imports, ordinary captures and budgeted crops retain their lease through
//! `TrackedImage`, including after
//! the admission service or original editor has gone away. Borrowing an Image
//! does not transfer ownership or detach its reservation. Explicitly cloning
//! that borrowed Image creates a new, independently untracked allocation.

use crate::{import_admission::TrackedImage, stitch::Image};
use std::{ops::Deref, sync::Arc};

#[derive(Clone, Debug)]
pub enum ImageHandle {
    Untracked(Arc<Image>),
    Imported(Arc<TrackedImage>),
}

impl ImageHandle {
    pub fn from_image(image: Image) -> Self {
        Self::Untracked(Arc::new(image))
    }

    pub fn from_arc(image: Arc<Image>) -> Self {
        Self::Untracked(image)
    }

    pub fn from_imported(image: Arc<TrackedImage>) -> Self {
        Self::Imported(image)
    }

    pub fn image(&self) -> &Image {
        match self {
            Self::Untracked(image) => image.as_ref(),
            Self::Imported(image) => image.image(),
        }
    }

    /// Whether both handles retain the same shared allocation.
    /// Equal pixel contents in distinct allocations do not imply identity.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Untracked(left), Self::Untracked(right)) => Arc::ptr_eq(left, right),
            (Self::Imported(left), Self::Imported(right)) => Arc::ptr_eq(left, right),
            _ => false,
        }
    }

    /// Allocation identity for deduplicating simultaneously retained images.
    /// This is not a persistent identifier: the address can be reused after
    /// every owner has dropped. The value must never be dereferenced.
    pub fn identity(&self) -> usize {
        std::ptr::from_ref(self.image()) as usize
    }
}

impl AsRef<Image> for ImageHandle {
    fn as_ref(&self) -> &Image {
        self.image()
    }
}

impl Deref for ImageHandle {
    type Target = Image;

    fn deref(&self) -> &Self::Target {
        self.image()
    }
}

// Preserve image-value comparison for document state equality. Allocation
// identity is deliberately explicit through ptr_eq/identity instead.
impl PartialEq for ImageHandle {
    fn eq(&self, other: &Self) -> bool {
        self.image() == other.image()
    }
}

impl Eq for ImageHandle {}

impl From<Image> for ImageHandle {
    fn from(image: Image) -> Self {
        Self::from_image(image)
    }
}

impl From<Arc<Image>> for ImageHandle {
    fn from(image: Arc<Image>) -> Self {
        Self::from_arc(image)
    }
}

impl From<Arc<TrackedImage>> for ImageHandle {
    fn from(image: Arc<TrackedImage>) -> Self {
        Self::from_imported(image)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        import_admission::ImportAdmission,
        resource_budget::{AdmissionClass, ResourceBudget},
    };

    fn image() -> Image {
        Image::new(1, 1, vec![3, 5, 7, 255]).unwrap()
    }

    fn imported(owner: &ImportAdmission) -> Arc<TrackedImage> {
        let image = image();
        let lease = owner
            .budget()
            .try_acquire(image.pixels.capacity(), AdmissionClass::Normal)
            .unwrap();
        owner.try_begin(image, lease).unwrap().complete()
    }

    #[test]
    fn untracked_construction_and_clone_keep_the_original_allocation() {
        let source = Arc::new(image());
        let pixels = source.pixels.as_ptr();
        let handle = ImageHandle::from_arc(Arc::clone(&source));
        let clone = handle.clone();
        assert_eq!(Arc::strong_count(&source), 3);
        assert!(handle.ptr_eq(&clone));
        assert_eq!(handle.identity(), clone.identity());
        assert!(std::ptr::eq(handle.as_ref(), source.as_ref()));
        assert_eq!(clone.pixels.as_ptr(), pixels);
        drop(source);
        drop(handle);
        assert_eq!(clone.pixels, [3, 5, 7, 255]);
    }

    #[test]
    fn owned_construction_moves_pixels_without_copying() {
        let source = image();
        let pixels = source.pixels.as_ptr();
        let handle = ImageHandle::from_image(source);
        assert_eq!(handle.pixels.as_ptr(), pixels);
    }

    #[test]
    fn imported_clone_keeps_pixels_and_lease_after_service_drop() {
        let budget = ResourceBudget::new(8, 0).unwrap();
        let owner = ImportAdmission::new(budget.clone());
        let source = imported(&owner);
        let pixels = source.image().pixels.as_ptr();
        let handle = ImageHandle::from_imported(Arc::clone(&source));
        let clone = handle.clone();
        assert_eq!(Arc::strong_count(&source), 3);
        assert!(handle.ptr_eq(&clone));
        assert_eq!(handle.identity(), clone.identity());
        assert!(std::ptr::eq(handle.as_ref(), source.image()));
        assert_eq!(clone.pixels.as_ptr(), pixels);
        drop(owner);
        drop(source);
        drop(handle);
        assert_eq!(budget.snapshot().used_bytes, 4);
        assert_eq!(clone.pixels, [3, 5, 7, 255]);
        drop(clone);
        assert_eq!(budget.snapshot().used_bytes, 0);
    }

    #[test]
    fn equal_contents_do_not_merge_distinct_or_cross_variant_allocations() {
        let owner = ImportAdmission::new(ResourceBudget::new(16, 0).unwrap());
        let plain = ImageHandle::from_image(image());
        let another_plain = ImageHandle::from_image(image());
        let tracked = ImageHandle::from_imported(imported(&owner));
        let another_tracked = ImageHandle::from_imported(imported(&owner));
        for other in [&another_plain, &tracked, &another_tracked] {
            assert_eq!(&plain, other);
            assert!(!plain.ptr_eq(other));
            assert_ne!(plain.identity(), other.identity());
        }
        assert_eq!(tracked, another_tracked);
        assert!(!tracked.ptr_eq(&another_tracked));
        assert_ne!(tracked.identity(), another_tracked.identity());
    }
}
