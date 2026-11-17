//! A stand-in for `fxc.exe`, the shader compiler from the Windows SDK.
//!
//! GPUI's build script runs fxc to compile its shaders into the executable
//! (`fxc /T <target> /E <entry> /Fh <out.h> /Vn <name> /O3 <file.hlsl>`).
//! Without them the app compiles the shaders every time it starts. The SDK is
//! a large download, but the compiler itself, d3dcompiler_47.dll, ships with
//! Windows, so this small program does the same job with it.
//!
//! Built by build.ps1 with plain `rustc`; it is not part of the notes crate.

use std::ffi::{CString, c_char, c_void};
use std::os::windows::ffi::OsStrExt;
use std::process::exit;

#[link(name = "d3dcompiler_47", kind = "raw-dylib")]
unsafe extern "system" {
    fn D3DCompileFromFile(
        file: *const u16,
        defines: *const c_void,
        include: *const c_void,
        entry: *const c_char,
        target: *const c_char,
        flags1: u32,
        flags2: u32,
        code: *mut *mut Blob,
        errors: *mut *mut Blob,
    ) -> i32;
}

/// `ID3DBlob`: a COM object holding a byte buffer.
#[repr(C)]
struct Blob {
    vtable: *const BlobVtable,
}

#[repr(C)]
struct BlobVtable {
    query_interface: *const c_void,
    add_ref: *const c_void,
    release: *const c_void,
    pointer: unsafe extern "system" fn(*mut Blob) -> *const u8,
    size: unsafe extern "system" fn(*mut Blob) -> usize,
}

/// # Safety
/// `blob` must be a live `ID3DBlob`.
unsafe fn bytes<'a>(blob: *mut Blob) -> &'a [u8] {
    unsafe {
        let vtable = &*(*blob).vtable;
        std::slice::from_raw_parts((vtable.pointer)(blob), (vtable.size)(blob))
    }
}

/// `D3D_COMPILE_STANDARD_FILE_INCLUDE`: resolve `#include` next to the file.
const STANDARD_FILE_INCLUDE: *const c_void = 1 as *const c_void;
/// `D3DCOMPILE_OPTIMIZATION_LEVEL3`, which is what `/O3` means.
const OPTIMIZATION_LEVEL3: u32 = 1 << 15;

fn fail(message: &str) -> ! {
    eprintln!("fxc: {message}");
    exit(1);
}

fn main() {
    let (mut target, mut entry, mut header, mut name, mut file) = (None, None, None, None, None);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| fail(&format!("{arg} needs a value")));
        match arg.as_str() {
            "/T" => target = Some(value()),
            "/E" => entry = Some(value()),
            "/Fh" => header = Some(value()),
            "/Vn" => name = Some(value()),
            "/O3" => {}
            other if other.starts_with('/') => fail(&format!("unsupported option {other}")),
            _ => file = Some(arg),
        }
    }
    let (Some(target), Some(entry), Some(header), Some(name), Some(file)) =
        (target, entry, header, name, file)
    else {
        fail("usage: fxc /T <target> /E <entry> /Fh <out.h> /Vn <name> /O3 <file.hlsl>");
    };

    let wide: Vec<u16> = std::ffi::OsStr::new(&file).encode_wide().chain(Some(0)).collect();
    let (entry_c, target_c) = (CString::new(entry.clone()).unwrap(), CString::new(target).unwrap());
    let (mut code, mut errors) = (std::ptr::null_mut(), std::ptr::null_mut());
    // SAFETY: every pointer is valid for the call, the strings are
    // NUL-terminated, and the blobs are only read when the API returned them.
    let compiled = unsafe {
        let result = D3DCompileFromFile(
            wide.as_ptr(),
            std::ptr::null(),
            STANDARD_FILE_INCLUDE,
            entry_c.as_ptr(),
            target_c.as_ptr(),
            OPTIMIZATION_LEVEL3,
            0,
            &mut code,
            &mut errors,
        );
        if result < 0 || code.is_null() {
            let detail = if errors.is_null() {
                format!("error {result:#x}")
            } else {
                String::from_utf8_lossy(bytes(errors)).into_owned()
            };
            fail(&format!("{file} ({entry}): {detail}"));
        }
        bytes(code)
    };

    // The same shape as fxc's /Fh output: a C array of the compiled bytes.
    let mut out = format!("const BYTE {name}[] =\n{{\n");
    for line in compiled.chunks(16) {
        let row: Vec<String> = line.iter().map(u8::to_string).collect();
        out.push_str(&format!("    {},\n", row.join(", ")));
    }
    out.push_str("};\n");
    if let Err(e) = std::fs::write(&header, out) {
        fail(&format!("cannot write {header}: {e}"));
    }
}
