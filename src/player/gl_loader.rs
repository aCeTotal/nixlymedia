use std::ffi::{c_void, CStr};
use std::os::raw::c_char;

use libloading::Library;
use once_cell::sync::Lazy;

static EGL: Lazy<Option<Library>> = Lazy::new(|| unsafe {
    Library::new("libEGL.so.1")
        .ok()
        .or_else(|| Library::new("libEGL.so").ok())
});

static GLX: Lazy<Option<Library>> = Lazy::new(|| unsafe {
    Library::new("libGL.so.1")
        .ok()
        .or_else(|| Library::new("libGL.so").ok())
});

#[cfg(windows)]
static OPENGL32: Lazy<Option<Library>> =
    Lazy::new(|| unsafe { Library::new("opengl32.dll").ok() });

pub unsafe fn get_proc(name: &CStr) -> *mut c_void {
    if let Some(lib) = EGL.as_ref() {
        if let Ok(sym) =
            lib.get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"eglGetProcAddress")
        {
            let p = sym(name.as_ptr());
            if !p.is_null() {
                return p;
            }
        }
    }
    if let Some(lib) = GLX.as_ref() {
        if let Ok(sym) =
            lib.get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"glXGetProcAddressARB")
        {
            let p = sym(name.as_ptr());
            if !p.is_null() {
                return p;
            }
        }
        if let Ok(sym) =
            lib.get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"glXGetProcAddress")
        {
            let p = sym(name.as_ptr());
            if !p.is_null() {
                return p;
            }
        }
        if let Ok(sym) = lib.get::<*mut c_void>(name.to_bytes_with_nul()) {
            return *sym;
        }
    }
    #[cfg(windows)]
    {
        if let Some(lib) = OPENGL32.as_ref() {
            if let Ok(sym) =
                lib.get::<unsafe extern "C" fn(*const c_char) -> *mut c_void>(b"wglGetProcAddress")
            {
                let p = sym(name.as_ptr());
                if !p.is_null() && (p as isize) != -1 {
                    return p;
                }
            }
            if let Ok(sym) = lib.get::<*mut c_void>(name.to_bytes_with_nul()) {
                return *sym;
            }
        }
    }
    std::ptr::null_mut()
}
