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

    // Only the client request struct starts from an empty slice.
    #[cfg(feature = "client")]
    #[inline]
    pub(crate) fn empty() -> Self {
        GearioHttpSlice {
            ptr: std::ptr::null(),
            len: 0,
        }
    }
}

/// One header, borrowed. Used by the client callbacks.
#[cfg(feature = "client")]
#[repr(C)]
pub struct GearioHttpHeader {
    pub name: GearioHttpSlice,
    pub value: GearioHttpSlice,
}

/// Why a request ended. Delivered to the client's on_done.
#[cfg(feature = "client")]
#[repr(C)]
pub struct GearioHttpError {
    /// One of the `GEARIO_HTTP_ERR_*` constants. The stable part.
    pub kind: crate::abi::GearioHttpErrorKind,
    /// Protocol-level code where one exists, else 0.
    pub protocol_code: u32,
    /// Borrowed diagnostic text. For logs only, never branch on it.
    pub message: GearioHttpSlice,
}
