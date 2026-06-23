//! Tiny env shim so the whole engine reads/writes config the same way on native AND wasm.
//!
//! martin's config is `MARTIN_*` environment variables (the `.show` + CLI expand into them; every
//! parser reads them). `wasm32-unknown-unknown` has **no process environment**: `std::env::var`
//! always returns `NotPresent` and `std::env::set_var` *panics* ("cannot set env vars on this
//! platform"). So on wasm we back the same get/set/remove with a process-global in-memory map,
//! seeded once in `main` (from the baked-in show). On native this is a zero-cost re-export of
//! `std::env`. The functions mirror `std::env`'s signatures exactly (incl. `set_var`/`remove_var`
//! being `unsafe`) so call sites are a pure `std::env::` → `crate::env::` rename.

#[cfg(not(target_arch = "wasm32"))]
pub use std::env::{remove_var, set_var, var, var_os};

#[cfg(target_arch = "wasm32")]
mod shim {
    use std::collections::BTreeMap;
    use std::env::VarError;
    use std::ffi::{OsStr, OsString};
    use std::sync::Mutex;

    // BTreeMap::new is const → a plain static, no OnceLock dance. Single-threaded on wasm anyway.
    static MAP: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

    fn key_of(k: impl AsRef<OsStr>) -> String {
        k.as_ref().to_string_lossy().into_owned()
    }

    pub fn var<K: AsRef<OsStr>>(key: K) -> Result<String, VarError> {
        MAP.lock()
            .unwrap()
            .get(&key_of(key))
            .cloned()
            .ok_or(VarError::NotPresent)
    }

    pub fn var_os<K: AsRef<OsStr>>(key: K) -> Option<OsString> {
        MAP.lock().unwrap().get(&key_of(key)).map(OsString::from)
    }

    /// # Safety
    /// Mirrors `std::env::set_var`'s unsafe signature; the in-memory map makes it actually safe.
    pub unsafe fn set_var<K: AsRef<OsStr>, V: AsRef<OsStr>>(key: K, value: V) {
        MAP.lock()
            .unwrap()
            .insert(key_of(key), value.as_ref().to_string_lossy().into_owned());
    }

    /// # Safety
    /// Mirrors `std::env::remove_var`'s unsafe signature; the in-memory map makes it actually safe.
    pub unsafe fn remove_var<K: AsRef<OsStr>>(key: K) {
        MAP.lock().unwrap().remove(&key_of(key));
    }
}

#[cfg(target_arch = "wasm32")]
pub use shim::{remove_var, set_var, var, var_os};
