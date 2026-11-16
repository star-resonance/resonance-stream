//! The game's IL2CPP embedding API, found by name with `GetProcAddress` on
//! `GameAssembly.dll` (never by signature scan), and [`Il2cppOps`] over it.
//!
//! The signatures are the public il2cpp embedding API as the author remembers it; they are
//! not in this repository and were not read from a header. NOT VERIFIED: il2cpp signatures
//! (Kade's in-game run settles them; a wrong one shows as a crash or a wrong probe line).

use super::{wide, GlobalLog};
use crate::error::{ProbeFailure, StepError};
use crate::log::LogSink;
use crate::names::{OPTIONAL_EXPORTS, REQUIRED_EXPORTS};
use crate::send_ops::SpanBytes;
use crate::session::Probe;
use crate::walk::{Arg, Exception, Handle, Il2cppOps, Walker};
use std::ffi::{c_char, c_void, CStr, CString};
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

type Ptr = *mut c_void;

type DomainGet = unsafe extern "C" fn() -> Ptr;
type DomainGetAssemblies = unsafe extern "C" fn(Ptr, *mut usize) -> *mut Ptr;
type AssemblyGetImage = unsafe extern "C" fn(Ptr) -> Ptr;
type ClassFromName = unsafe extern "C" fn(Ptr, *const c_char, *const c_char) -> Ptr;
type ClassGetParent = unsafe extern "C" fn(Ptr) -> Ptr;
type ClassGetMethodFromName = unsafe extern "C" fn(Ptr, *const c_char, i32) -> Ptr;
type ClassGetFieldFromName = unsafe extern "C" fn(Ptr, *const c_char) -> Ptr;
type FieldGetOffset = unsafe extern "C" fn(Ptr) -> usize;
type FieldStaticGetValue = unsafe extern "C" fn(Ptr, Ptr);
type RuntimeInvoke = unsafe extern "C" fn(Ptr, Ptr, *mut Ptr, *mut Ptr) -> Ptr;
type ObjectGetClass = unsafe extern "C" fn(Ptr) -> Ptr;
type ThreadAttach = unsafe extern "C" fn(Ptr) -> Ptr;
type ClassGetText = unsafe extern "C" fn(Ptr) -> *const c_char;
type ThreadDetach = unsafe extern "C" fn(Ptr);

/// The resolved exports. The twelve required ones are all there or [`Api::load`] fails.
#[allow(dead_code)] // field_static_get_value is resolved (the doc lists it) but not called yet
pub struct Api {
    domain_get: DomainGet,
    domain_get_assemblies: DomainGetAssemblies,
    assembly_get_image: AssemblyGetImage,
    class_from_name: ClassFromName,
    class_get_parent: ClassGetParent,
    class_get_method_from_name: ClassGetMethodFromName,
    class_get_field_from_name: ClassGetFieldFromName,
    field_get_offset: FieldGetOffset,
    field_static_get_value: FieldStaticGetValue,
    runtime_invoke: RuntimeInvoke,
    object_get_class: ObjectGetClass,
    thread_attach: ThreadAttach,
    class_get_namespace: Option<ClassGetText>,
    class_get_name: Option<ClassGetText>,
    thread_detach: Option<ThreadDetach>,
}

/// `GetProcAddress` as a function pointer of type `F`.
///
/// # Safety
/// `F` must be the `extern "C"` function pointer type the export really has.
unsafe fn proc<F: Copy>(module: HMODULE, name: &str) -> Option<F> {
    assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<usize>());
    let name = CString::new(name).ok()?;
    // SAFETY: `module` is a loaded module and `name` is NUL-terminated.
    let address = unsafe { GetProcAddress(module, name.as_ptr().cast()) }?;
    // SAFETY: the caller promises `F` is the export's type; both are pointer-sized.
    Some(unsafe { std::mem::transmute_copy::<_, F>(&address) })
}

impl Api {
    /// Resolves the exports of `GameAssembly.dll`; logs each one found or missing.
    pub fn load(log: &mut dyn LogSink) -> Result<Api, StepError> {
        let name = wide("GameAssembly.dll");
        // SAFETY: `name` is a NUL-terminated wide string.
        let module = unsafe { GetModuleHandleW(name.as_ptr()) };
        if module.is_null() {
            return Err(StepError::export("GameAssembly.dll"));
        }
        for name in OPTIONAL_EXPORTS {
            // SAFETY: only presence is checked here; the type is not used.
            let found = unsafe { proc::<usize>(module, name) }.is_some();
            log.info(&format!(
                "optional export {name} {}",
                if found { "found" } else { "missing" }
            ));
        }
        macro_rules! required {
            ($index:expr, $ty:ty) => {{
                let name = REQUIRED_EXPORTS[$index];
                // SAFETY: the table pairs each name with its public signature.
                let f = unsafe { proc::<$ty>(module, name) };
                log.info(&format!(
                    "export {name} {}",
                    if f.is_some() { "found" } else { "missing" }
                ));
                f.ok_or_else(|| StepError::export(name))?
            }};
        }
        let api = Api {
            domain_get: required!(0, DomainGet),
            domain_get_assemblies: required!(1, DomainGetAssemblies),
            assembly_get_image: required!(2, AssemblyGetImage),
            class_from_name: required!(3, ClassFromName),
            class_get_parent: required!(4, ClassGetParent),
            class_get_method_from_name: required!(5, ClassGetMethodFromName),
            class_get_field_from_name: required!(6, ClassGetFieldFromName),
            field_get_offset: required!(7, FieldGetOffset),
            field_static_get_value: required!(8, FieldStaticGetValue),
            runtime_invoke: required!(9, RuntimeInvoke),
            object_get_class: required!(10, ObjectGetClass),
            thread_attach: required!(11, ThreadAttach),
            // SAFETY: as above, for the optional table.
            class_get_namespace: unsafe { proc(module, OPTIONAL_EXPORTS[0]) },
            class_get_name: unsafe { proc(module, OPTIONAL_EXPORTS[1]) },
            thread_detach: unsafe { proc(module, OPTIONAL_EXPORTS[2]) },
        };
        Ok(api)
    }
}

/// [`Il2cppOps`] over the real game.
pub struct Il2cpp {
    api: Api,
    domain: Ptr,
    thread: Ptr,
}

impl Il2cpp {
    /// Attaches the calling thread to the il2cpp domain (never from `DllMain`).
    pub fn attach(api: Api) -> Il2cpp {
        // SAFETY: `domain_get` and `thread_attach` are the il2cpp exports of that name.
        let domain = unsafe { (api.domain_get)() };
        let thread = unsafe { (api.thread_attach)(domain) };
        Il2cpp {
            api,
            domain,
            thread,
        }
    }

    /// Detaches the thread attached by [`Il2cpp::attach`], if the game exports detach.
    pub fn detach(&mut self) {
        if let Some(detach) = self.api.thread_detach {
            if !self.thread.is_null() {
                // SAFETY: `thread` came from `il2cpp_thread_attach` on this thread.
                unsafe { detach(self.thread) };
                self.thread = std::ptr::null_mut();
            }
        }
    }

    fn text(f: Option<ClassGetText>, class: Handle) -> Option<String> {
        let f = f?;
        // SAFETY: `class` is a class pointer; the result is a NUL-terminated string owned by il2cpp.
        let p = unsafe { f(class as Ptr) };
        if p.is_null() {
            return None;
        }
        // SAFETY: non-null, NUL-terminated, valid for this call.
        Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
    }
}

impl Il2cppOps for Il2cpp {
    fn class_from_name(&mut self, namespace: &str, name: &str) -> Option<Handle> {
        let (namespace, name) = (CString::new(namespace).ok()?, CString::new(name).ok()?);
        let mut count = 0usize;
        // SAFETY: `domain` is the il2cpp domain; `count` receives the length of the array.
        let assemblies = unsafe { (self.api.domain_get_assemblies)(self.domain, &mut count) };
        if assemblies.is_null() {
            return None;
        }
        for i in 0..count {
            // SAFETY: `assemblies` points to `count` assembly pointers.
            let assembly = unsafe { *assemblies.add(i) };
            // SAFETY: `assembly` came from the domain; the image and class are il2cpp's.
            let image = unsafe { (self.api.assembly_get_image)(assembly) };
            if image.is_null() {
                continue;
            }
            let class =
                unsafe { (self.api.class_from_name)(image, namespace.as_ptr(), name.as_ptr()) };
            if !class.is_null() {
                return Some(class as Handle);
            }
        }
        None
    }

    fn class_parent(&mut self, class: Handle) -> Option<Handle> {
        // SAFETY: `class` is a class pointer from this API.
        let parent = unsafe { (self.api.class_get_parent)(class as Ptr) };
        (!parent.is_null()).then_some(parent as Handle)
    }

    fn class_name(&mut self, class: Handle) -> Option<String> {
        let name = Self::text(self.api.class_get_name, class)?;
        match Self::text(self.api.class_get_namespace, class) {
            Some(ns) if !ns.is_empty() => Some(format!("{ns}.{name}")),
            _ => Some(name),
        }
    }

    fn method(&mut self, class: Handle, name: &str, argc: i32) -> Option<Handle> {
        let name = CString::new(name).ok()?;
        // SAFETY: `class` is a class pointer; `name` is NUL-terminated.
        let m = unsafe { (self.api.class_get_method_from_name)(class as Ptr, name.as_ptr(), argc) };
        (!m.is_null()).then_some(m as Handle)
    }

    fn field(&mut self, class: Handle, name: &str) -> Option<Handle> {
        let name = CString::new(name).ok()?;
        // SAFETY: as in `method`.
        let f = unsafe { (self.api.class_get_field_from_name)(class as Ptr, name.as_ptr()) };
        (!f.is_null()).then_some(f as Handle)
    }

    fn field_offset(&mut self, field: Handle) -> usize {
        // SAFETY: `field` is a field pointer from this API.
        unsafe { (self.api.field_get_offset)(field as Ptr) }
    }

    fn object_class(&mut self, object: usize) -> Handle {
        // SAFETY: `object` is a managed object the walk got from the game.
        unsafe { (self.api.object_get_class)(object as Ptr) as Handle }
    }

    fn invoke(&mut self, method: Handle, this: usize, args: &[Arg]) -> Result<usize, Exception> {
        // A value-type argument is passed as a pointer to the value; the values live in
        // these vectors until the call returns.
        let numbers: Vec<u64> = args
            .iter()
            .map(|a| match a {
                Arg::U64(v) => *v,
                Arg::Span { .. } => 0,
            })
            .collect();
        let spans: Vec<SpanBytes> = args
            .iter()
            .map(|a| match a {
                Arg::Span { ptr, len } => SpanBytes {
                    reference: *ptr as *const u8,
                    length: *len as i32,
                    _pad: 0,
                },
                Arg::U64(_) => SpanBytes::over(&[]),
            })
            .collect();
        let mut params: Vec<Ptr> = args
            .iter()
            .enumerate()
            .map(|(i, a)| match a {
                Arg::U64(_) => &numbers[i] as *const u64 as Ptr,
                Arg::Span { .. } => &spans[i] as *const SpanBytes as Ptr,
            })
            .collect();
        let mut exception: Ptr = std::ptr::null_mut();
        // SAFETY: `method` is a method pointer, `this` is null or an object of its class,
        // `params` holds one pointer per argument and outlives the call.
        let result = unsafe {
            (self.api.runtime_invoke)(
                method as Ptr,
                this as Ptr,
                params.as_mut_ptr(),
                &mut exception,
            )
        };
        if exception.is_null() {
            Ok(result as usize)
        } else {
            Err(Exception)
        }
    }

    fn method_pointer(&mut self, method: Handle) -> usize {
        // SAFETY: `method` is a `MethodInfo*` from `il2cpp_class_get_method_from_name`; its
        // first field is the native code pointer (public il2cpp layout, estimated, NOT
        // VERIFIED: it is not in this repository and was not read from a header).
        unsafe { std::ptr::read_unaligned(method as *const usize) }
    }

    fn read_ptr(&mut self, address: usize) -> usize {
        // SAFETY: `address` is an object the game returned plus a field offset the game
        // reported; the read is unaligned-safe.
        unsafe { std::ptr::read_unaligned(address as *const usize) }
    }
}

/// The probe the session runs: the walk over the real game, or the resolution error that
/// every probe repeats.
pub enum GameProbe {
    Ready(Walker<Il2cpp>),
    Failed(StepError),
}

impl GameProbe {
    /// Resolves the exports and attaches the thread; a failure is logged once.
    pub fn open(log: &mut dyn LogSink) -> GameProbe {
        match Api::load(log) {
            Ok(api) => GameProbe::Ready(Walker::new(Il2cpp::attach(api))),
            Err(e) => {
                log.warn(&format!("resolution failed: {e}"));
                GameProbe::Failed(e)
            }
        }
    }

    pub fn detach(&mut self) {
        if let GameProbe::Ready(w) = self {
            w.ops_mut().detach();
        }
    }
}

impl Probe for GameProbe {
    fn probe(&mut self) -> Result<(), ProbeFailure> {
        match self {
            GameProbe::Ready(w) => w.walk(&mut GlobalLog).map(|_| ()),
            GameProbe::Failed(e) => Err(ProbeFailure::Unsupported(e.clone())),
        }
    }
}
