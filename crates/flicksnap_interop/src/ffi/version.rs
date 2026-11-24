//! What this library is: the ABI it speaks and the version it was built from (README E).

use crate::error::Result;
use crate::ffi::status::FLICKSNAP_OK;
use crate::ffi::types::FlickSnapString;
use crate::{panic, util};

/// This library's version: `FLICKSNAP_VERSION` when the build stamped one (CI and build.sh do, the
/// same value flicksnap_telemetry's release uses), else `CARGO_PKG_VERSION`.
pub(crate) fn version() -> &'static str {
    match option_env!("FLICKSNAP_VERSION") {
        Some(stamped) if !stamped.is_empty() => stamped,
        _ => env!("CARGO_PKG_VERSION"),
    }
}

/// Writes the ABI fingerprint of this build to `out_hash`: a 64-bit FNV-1a over every export
/// signature, `#[repr(C)]` struct and `#[repr]` enum in `src/ffi` and the hotkey key table
/// (README E1). The C# loader calls this through `NativeLibrary.TryGetExport` before any
/// DllImport resolves, and refuses a library whose value differs from the generated
/// `InteropAbi.Hash` ("stale bindings: run cargo codegen").
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` for a null `out_hash`.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_interop_abi_hash(out_hash: *mut u64) -> i32 {
    panic::guard("flicksnap_interop_abi_hash", || abi_hash(out_hash))
}

unsafe fn abi_hash(out_hash: *mut u64) -> Result {
    util::write(out_hash, "out_hash", crate::ABI_HASH)?;
    Ok(FLICKSNAP_OK)
}

/// Writes this library's version string to `out_version` as a standalone owned string (mode 1:
/// free it with `flicksnap_string_free`). `FLICKSNAP_VERSION` stamped at build time, `1.0.0` in a plain
/// `cargo build`.
///
/// Status: `FLICKSNAP_OK`; `FLICKSNAP_E_INVALID_ARG` for a null `out_version`.
///
/// Thread: any thread.
#[no_mangle]
pub unsafe extern "C" fn flicksnap_interop_version(out_version: *mut FlickSnapString) -> i32 {
    panic::guard("flicksnap_interop_version", || write_version(out_version))
}

unsafe fn write_version(out_version: *mut FlickSnapString) -> Result {
    util::write(out_version, "out_version", FlickSnapString::from_string(version()))?;
    Ok(FLICKSNAP_OK)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffi::status::FlickSnapInteropStatus;

    #[test]
    fn abi_hash_is_the_built_fingerprint() {
        let mut hash = 0u64;
        assert_eq!(unsafe { flicksnap_interop_abi_hash(&mut hash) }, 0);
        assert_eq!(hash, crate::ABI_HASH);
        assert_ne!(hash, 0);
        assert_eq!(
            unsafe { flicksnap_interop_abi_hash(std::ptr::null_mut()) },
            FlickSnapInteropStatus::InvalidArg as i32
        );
    }

    #[test]
    fn version_is_an_owned_string() {
        let mut s = FlickSnapString::empty();
        assert_eq!(unsafe { flicksnap_interop_version(&mut s) }, 0);
        assert_eq!(unsafe { s.as_str() }, version());
        unsafe { s.release() };
    }
}
