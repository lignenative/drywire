//! Protects the secrets the app keeps on disk: the device's identity and
//! the space keys from invitation links (ADR 0031).
//!
//! - **Windows**: DPAPI (`CryptProtectData`, user scope). Only this
//!   Windows user, on this machine, can open what it seals; a copy of the
//!   file taken elsewhere, or read by another user, is useless.
//! - **Elsewhere**: no protection beyond a file only its owner can read.
//!   The desktop app ships on Windows first; on macOS it runs for
//!   development only, and Linux gets the Secret Service before it ships.
//!
//! A sealed value is stored with the name of what protects it, so that a
//! file sealed by DPAPI is recognised, and refused clearly, elsewhere.

/// What protects the secrets on this system, as written in the files.
#[cfg(windows)]
pub const PROTECTION: &str = "dpapi";
/// What protects the secrets on this system, as written in the files.
#[cfg(not(windows))]
pub const PROTECTION: &str = "none";

/// Seals `plain` for this user.
pub fn seal(plain: &[u8]) -> Result<Vec<u8>, String> {
    #[cfg(windows)]
    {
        dpapi::protect(plain)
    }
    #[cfg(not(windows))]
    {
        Ok(plain.to_vec())
    }
}

/// Opens what [`seal`] sealed, under the protection named in the file.
pub fn open(protection: &str, sealed: &[u8]) -> Result<Vec<u8>, String> {
    if protection != PROTECTION {
        return Err(format!(
            "this file is protected with {protection:?}, which this system cannot open \
             (it uses {PROTECTION:?}): it was copied from another system"
        ));
    }
    #[cfg(windows)]
    {
        dpapi::unprotect(sealed)
    }
    #[cfg(not(windows))]
    {
        Ok(sealed.to_vec())
    }
}

#[cfg(windows)]
mod dpapi {
    use windows::Win32::Foundation::{HLOCAL, LocalFree};
    use windows::Win32::Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    };
    use windows::core::PCWSTR;

    /// Mixed into every blob: another program of the same user has to know
    /// it to open ours. It is not a secret, only a namespace.
    const ENTROPY: &[u8] = b"drywire-desktop vault v1";

    fn blob(bytes: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            // DPAPI only reads the input blobs.
            pbData: bytes.as_ptr().cast_mut(),
        }
    }

    /// Copies DPAPI's output and frees it with `LocalFree`, as its
    /// documentation requires.
    ///
    /// # Safety
    ///
    /// `out` must have been filled by a successful DPAPI call.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // SAFETY: DPAPI wrote `cbData` bytes at `pbData` (caller's promise).
        let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        // SAFETY: `pbData` was allocated by DPAPI with LocalAlloc, and is
        // freed once, here.
        unsafe { LocalFree(Some(HLOCAL(out.pbData.cast()))) };
        bytes
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(plain);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: every pointer is valid for the call: the input blobs
        // point into live slices, and `out` is ours to fill.
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(|e| format!("DPAPI could not protect a secret: {e}"))?;
        // SAFETY: the call succeeded.
        Ok(unsafe { take(out) })
    }

    pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(sealed);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `protect`.
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(|e| {
            format!(
                "DPAPI could not open a secret: it belongs to another Windows user or \
                 machine, or it is damaged ({e})"
            )
        })?;
        // SAFETY: the call succeeded.
        Ok(unsafe { take(out) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_sealed_opens_again() {
        let secret = b"a space key, or a seed";
        let sealed = seal(secret).unwrap();
        if cfg!(windows) {
            assert_ne!(sealed, secret, "DPAPI must not store the secret as is");
        }
        assert_eq!(open(PROTECTION, &sealed).unwrap(), secret);
    }

    #[test]
    fn a_file_from_another_system_is_refused() {
        let other = if PROTECTION == "dpapi" {
            "none"
        } else {
            "dpapi"
        };
        let err = open(other, b"whatever").unwrap_err();
        assert!(err.contains("another system"), "{err}");
    }
}
