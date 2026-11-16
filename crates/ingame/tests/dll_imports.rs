//! The shipped DLL must import no C-runtime DLL: the game process need not hold a VC runtime.
//!
//! `imported_dlls` reads the import and delay-import tables of a PE32+ file (std only, every
//! read bounds-checked); `check_imports` applies the rule. The real-DLL test is `#[ignore]`
//! (it needs the built file); `just ingame-build` runs it with `--ignored --nocapture`, which
//! prints the imports one per line.

use std::path::PathBuf;

/// Names of the DLLs the PE file imports (import table, then delay-import table).
fn imported_dlls(_bytes: &[u8]) -> Result<Vec<String>, String> {
    Err("not implemented".into())
}

/// True for a C-runtime DLL (compared lowercased).
fn forbidden_import(_name: &str) -> bool {
    false
}

/// `Ok` when no import is a C-runtime DLL and the list holds `kernel32.dll` (a parser that
/// found nothing must not pass).
fn check_imports(_names: &[String]) -> Result<(), String> {
    Err("not implemented".into())
}

// ---- fixture: a minimal PE32+ with one `.rdata` section ----

const SECTION_VA: u32 = 0x1000;
const SECTION_RAW: usize = 0x200;

fn put16(b: &mut [u8], at: usize, v: u16) {
    b[at..at + 2].copy_from_slice(&v.to_le_bytes());
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// A PE with the given import and delay-import DLL names (an empty list: no directory).
fn pe(imports: &[&str], delay: &[&str]) -> Vec<u8> {
    let e_lfanew = 0x80usize;
    let opt = e_lfanew + 24;
    let opt_size = 240usize;
    let sect = opt + opt_size;
    // section body: descriptors first, names after
    let imp_len = if imports.is_empty() {
        0
    } else {
        (imports.len() + 1) * 20
    };
    let del_len = if delay.is_empty() {
        0
    } else {
        (delay.len() + 1) * 32
    };
    let mut body = vec![0u8; imp_len + del_len];
    let mut name_rvas = Vec::new();
    for n in imports.iter().chain(delay.iter()) {
        name_rvas.push(SECTION_VA + body.len() as u32);
        body.extend_from_slice(n.as_bytes());
        body.push(0);
    }
    for i in 0..imports.len() {
        put32(&mut body, i * 20 + 12, name_rvas[i]);
    }
    for i in 0..delay.len() {
        put32(
            &mut body,
            imp_len + i * 32 + 4,
            name_rvas[imports.len() + i],
        );
    }
    let mut f = vec![0u8; SECTION_RAW];
    f[0] = b'M';
    f[1] = b'Z';
    put32(&mut f, 0x3C, e_lfanew as u32);
    f[e_lfanew..e_lfanew + 4].copy_from_slice(b"PE\0\0");
    put16(&mut f, e_lfanew + 4, 0x8664);
    put16(&mut f, e_lfanew + 6, 1); // NumberOfSections
    put16(&mut f, e_lfanew + 20, opt_size as u16);
    put16(&mut f, opt, 0x20B);
    if !imports.is_empty() {
        put32(&mut f, opt + 112 + 8, SECTION_VA);
        put32(&mut f, opt + 112 + 12, imp_len as u32);
    }
    if !delay.is_empty() {
        put32(&mut f, opt + 112 + 13 * 8, SECTION_VA + imp_len as u32);
        put32(&mut f, opt + 112 + 13 * 8 + 4, del_len as u32);
    }
    f[sect..sect + 6].copy_from_slice(b".rdata");
    put32(&mut f, sect + 8, body.len() as u32);
    put32(&mut f, sect + 12, SECTION_VA);
    put32(&mut f, sect + 16, body.len() as u32);
    put32(&mut f, sect + 20, SECTION_RAW as u32);
    f.extend_from_slice(&body);
    f
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

// (a) parser

#[test]
fn reads_imports_in_order() {
    let got = imported_dlls(&pe(&["KERNEL32.dll", "VCRUNTIME140.dll"], &[]));
    assert_eq!(got, Ok(names(&["KERNEL32.dll", "VCRUNTIME140.dll"])));
}

#[test]
fn delay_imports_count() {
    let got = imported_dlls(&pe(&["kernel32.dll"], &["VCRUNTIME140.dll"]));
    assert_eq!(got, Ok(names(&["kernel32.dll", "VCRUNTIME140.dll"])));
}

#[test]
fn no_import_directory_is_empty() {
    assert_eq!(imported_dlls(&pe(&[], &[])), Ok(vec![]));
}

#[test]
fn malformed_files_are_errors_not_panics() {
    let good = pe(&["kernel32.dll"], &[]);
    assert!(imported_dlls(&[]).is_err());
    // cut inside the section table
    assert!(imported_dlls(&good[..0x80 + 24 + 240 + 10]).is_err());
    // e_lfanew past the end
    let mut b = good.clone();
    put32(&mut b, 0x3C, 0xFFFF_0000);
    assert!(imported_dlls(&b).is_err());
    // missing "PE\0\0"
    let mut b = good.clone();
    b[0x80] = b'X';
    assert!(imported_dlls(&b).is_err());
    // PE32 magic
    let mut b = good.clone();
    put16(&mut b, 0x80 + 24, 0x10B);
    assert!(imported_dlls(&b).is_err());
    // name RVA outside every section
    let mut b = good.clone();
    put32(&mut b, SECTION_RAW + 12, 0x9000_0000);
    assert!(imported_dlls(&b).is_err());
    // name without terminator
    let mut b = good.clone();
    let n = b.len();
    b[n - 1] = b'x';
    assert!(imported_dlls(&b).is_err());
}

// (b) rule

#[test]
fn forbidden_names() {
    for n in [
        "VCRUNTIME140.dll",
        "vcruntime140_1.dll",
        "MSVCP140.dll",
        "msvcr120.dll",
        "UCRTBASE.dll",
        "api-ms-win-crt-heap-l1-1-0.dll",
    ] {
        assert!(forbidden_import(n), "{n}");
    }
    for n in [
        "KERNEL32.dll",
        "ntdll.dll",
        "api-ms-win-core-synch-l1-2-0.dll",
        "bcryptprimitives.dll",
        "psapi.dll",
        "version.dll",
    ] {
        assert!(!forbidden_import(n), "{n}");
    }
}

// (c) check

#[test]
fn check_rejects_crt_and_empty_and_no_kernel32() {
    let e = check_imports(&names(&["kernel32.dll", "VCRUNTIME140.dll"])).unwrap_err();
    assert!(e.contains("VCRUNTIME140.dll"), "{e}");
    assert!(check_imports(&[]).is_err());
    assert!(check_imports(&names(&["ntdll.dll"])).is_err());
}

#[test]
fn check_accepts_the_static_build_imports() {
    let ok = names(&[
        "kernel32.dll",
        "version.dll",
        "api-ms-win-core-synch-l1-2-0.dll",
        "psapi.dll",
        "ntdll.dll",
        "bcryptprimitives.dll",
    ]);
    assert_eq!(check_imports(&ok), Ok(()));
}

// (d) the built DLL

fn dll_path() -> PathBuf {
    if let Some(p) = std::env::var_os("RESONANCE_INGAME_DLL") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/x86_64-pc-windows-msvc/ingame/resonance_ingame.dll")
}

#[test]
#[ignore = "needs the built DLL: run by `just ingame-build`"]
fn built_dll_imports_no_c_runtime() {
    let path = dll_path();
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let names = imported_dlls(&bytes).unwrap();
    for n in &names {
        println!("{n}");
    }
    check_imports(&names).unwrap();
}
