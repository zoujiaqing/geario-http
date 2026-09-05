//! Borrowed byte views handed to C.

/// A borrowed view of bytes owned by geario-http.
///
/// Only valid for the duration of the callback it arrives in. A host that
/// needs the bytes afterwards must copy them.
#[repr(C)]
pub struct GearioHttpSlice {
    pub ptr: *const u8,
    pub len: usize,
}

impl GearioHttpSlice {
    #[inline]
    pub(crate) fn borrow(b: &[u8]) -> Self {
        GearioHttpSlice {
            ptr: b.as_ptr(),
            len: b.len(),
        }
    }

    #[inline]
    pub(crate) fn empty() -> Self {
        GearioHttpSlice {
            ptr: std::ptr::null(),
            len: 0,
        }
    }
}
