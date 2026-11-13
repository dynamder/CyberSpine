//! Packaging an arbiter as a dynamic library.
//!
//! [`export_arbiter!`] writes the single C symbol a host resolves, an
//! [`AbiV1`](crate::abi::AbiV1) table over [`server::Server`], and a panic guard
//! on every entry. An arbiter written for the linked mode needs no other change:
//! the same `impl Arbiter` is the thing that ships.

/// Export an arbiter from a `cdylib`.
///
/// ```ignore
/// cyberspine_arbiter::export_arbiter! {
///     arbiter: Scripted,
///     create: |_config| Scripted::new(),
/// }
/// ```
///
/// * `arbiter` — the type that implements [`Arbiter`](crate::Arbiter).
/// * `create` — a closure `FnOnce(Option<&str>) -> Arbiter`; the `&str` is the
///   host's config argument, if it passed one.
/// * `build_id` — optional; defaults to the package name and version, and is
///   reported to the host for run records.
///
/// The macro expands to a module named `__csp_export`, so one arbiter per crate.
#[macro_export]
macro_rules! export_arbiter {
    (arbiter: $arbiter:ty, create: $create:expr $(,)?) => {
        $crate::export_arbiter!(@build
            arbiter: $arbiter,
            create: $create,
            build_id: ::core::concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")),
        );
    };
    (arbiter: $arbiter:ty, create: $create:expr, build_id: $build_id:expr $(,)?) => {
        $crate::export_arbiter!(@build
            arbiter: $arbiter,
            create: $create,
            build_id: $build_id,
        );
    };
    (@build arbiter: $arbiter:ty, create: $create:expr, build_id: $build_id:expr $(,)?) => {
        #[doc(hidden)]
        #[allow(non_snake_case)]
        pub mod __csp_export {
            use super::*;
            use ::std::any::Any;
            use ::std::ffi::{c_char, c_void, CStr, CString};
            use ::std::panic::{AssertUnwindSafe, catch_unwind};
            use ::std::ptr;

            use $crate::abi::{self, AbiV1, status};
            use $crate::server::Server;

            const BUILD_ID: &str = $build_id;

            struct Handle {
                server: Option<Server<$arbiter>>,
                error: Option<CString>,
            }

            unsafe extern "C" fn create(config: *const c_char) -> *mut c_void {
                let config = if config.is_null() {
                    None
                } else {
                    match unsafe { CStr::from_ptr(config) }.to_str() {
                        Ok(text) => Some(text.to_owned()),
                        Err(_) => {
                            eprintln!("csp: the config argument is not UTF-8");
                            return ptr::null_mut();
                        }
                    }
                };

                let made = catch_unwind(AssertUnwindSafe(|| {
                    let arbiter: $arbiter = ($create)(config.as_deref());
                    Server::new(arbiter, BUILD_ID.to_owned())
                }));

                let (server, error) = match made {
                    Ok(server) => (Some(server), None),
                    Err(payload) => (None, Some(panic_text(payload))),
                };
                Box::into_raw(Box::new(Handle { server, error })) as *mut c_void
            }

            unsafe extern "C" fn destroy(arbiter: *mut c_void) {
                if arbiter.is_null() {
                    return;
                }
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    drop(unsafe { Box::from_raw(arbiter as *mut Handle) });
                }));
            }

            unsafe extern "C" fn call(
                arbiter: *mut c_void,
                request: *const u8,
                request_len: usize,
                response: *mut u8,
                response_cap: usize,
                response_len: *mut usize,
            ) -> i32 {
                if arbiter.is_null() || response_len.is_null() || response.is_null() {
                    return status::PROTOCOL;
                }
                let handle = unsafe { &mut *(arbiter as *mut Handle) };

                let request = if request_len == 0 {
                    &[][..]
                } else if request.is_null() {
                    unsafe { *response_len = 0 };
                    return status::PROTOCOL;
                } else {
                    unsafe { ::std::slice::from_raw_parts(request, request_len) }
                };

                let Some(server) = handle.server.as_mut() else {
                    unsafe { *response_len = 0 };
                    return status::ARBITER;
                };

                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    let reply = server.answer(request);
                    if reply.len() > response_cap {
                        return Err(reply.len());
                    }
                    unsafe { ptr::copy_nonoverlapping(reply.as_ptr(), response, reply.len()) };
                    Ok(reply.len())
                }));

                match outcome {
                    Ok(Ok(len)) => {
                        handle.error = None;
                        unsafe { *response_len = len };
                        status::OK
                    }
                    Ok(Err(needed)) => {
                        unsafe { *response_len = needed };
                        status::RESPONSE_TOO_SMALL
                    }
                    Err(payload) => {
                        handle.error = Some(panic_text(payload));
                        status::PANIC
                    }
                }
            }

            unsafe extern "C" fn last_error(arbiter: *mut c_void) -> *const c_char {
                if arbiter.is_null() {
                    return ptr::null();
                }
                let handle = unsafe { &*(arbiter as *const Handle) };
                match &handle.error {
                    Some(error) => error.as_ptr(),
                    None => ptr::null(),
                }
            }

            fn text(message: String) -> CString {
                CString::new(message).unwrap_or_default()
            }

            fn panic_text(payload: Box<dyn Any + Send>) -> CString {
                let message = payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "the arbiter panicked".to_owned());
                text(message)
            }

            pub static TABLE: AbiV1 = AbiV1 {
                version: abi::ABI_VERSION,
                size: ::core::mem::size_of::<AbiV1>() as u32,
                create,
                destroy,
                call,
                last_error,
            };
        }

        #[unsafe(no_mangle)]
        pub extern "C" fn csp_arbiter_v1() -> *const $crate::abi::AbiV1 {
            &__csp_export::TABLE
        }
    };
}
