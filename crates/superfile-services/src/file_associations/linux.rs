//! Query GIO's effective defaults; missing native libraries produce a recoverable error.
#![allow(unsafe_code)]
use super::*;
use anyhow::{Context, anyhow};
use libloading::Library;
use std::io::Read;
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_void},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    sync::OnceLock,
};

#[repr(C)]
struct List {
    data: *mut c_void,
    next: *mut List,
    previous: *mut List,
}
#[repr(C)]
struct GioError {
    domain: u32,
    code: i32,
    message: *mut c_char,
}
type Free = unsafe extern "C" fn(*mut c_void);
struct Gio {
    _libraries: [Library; 3],
    types: unsafe extern "C" fn() -> *mut List,
    guess: unsafe extern "C" fn(*const c_char, *const u8, usize, *mut i32) -> *mut c_char,
    default: unsafe extern "C" fn(*const c_char, i32) -> *mut c_void,
    name: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    identifier: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    executable: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    description: unsafe extern "C" fn(*const c_char) -> *mut c_char,
    free: Free,
    unref: Free,
    free_list: unsafe extern "C" fn(*mut List, Option<Free>),
    applications: unsafe extern "C" fn(*const c_char) -> *mut List,
    set_default: unsafe extern "C" fn(*mut c_void, *const c_char, *mut *mut GioError) -> i32,
    free_error: unsafe extern "C" fn(*mut GioError),
}
impl Gio {
    fn load() -> Result<Self> {
        // Only system library names are used. Libraries remain loaded for the process lifetime.
        unsafe {
            let glib = Library::new("libglib-2.0.so.0")
                .context("GLib is required to read system file associations")?;
            let object = Library::new("libgobject-2.0.so.0")?;
            let gio = Library::new("libgio-2.0.so.0")
                .context("GIO is required to read system file associations")?;
            Ok(Self {
                types: *gio.get(b"g_content_types_get_registered\0")?,
                guess: *gio.get(b"g_content_type_guess\0")?,
                default: *gio.get(b"g_app_info_get_default_for_type\0")?,
                name: *gio.get(b"g_app_info_get_display_name\0")?,
                identifier: *gio.get(b"g_app_info_get_id\0")?,
                executable: *gio.get(b"g_app_info_get_executable\0")?,
                description: *gio.get(b"g_content_type_get_description\0")?,
                free: *glib.get(b"g_free\0")?,
                unref: *object.get(b"g_object_unref\0")?,
                free_list: *glib.get(b"g_list_free_full\0")?,
                applications: *gio.get(b"g_app_info_get_all_for_type\0")?,
                set_default: *gio.get(b"g_app_info_set_as_default_for_type\0")?,
                free_error: *glib.get(b"g_error_free\0")?,
                _libraries: [gio, object, glib],
            })
        }
    }
}

fn string(value: *const c_char) -> Option<String> {
    (!value.is_null())
        .then(|| {
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned()
        })
        .filter(|value| !value.is_empty())
}
struct Types<'a>(*mut List, &'a Gio, Free);
impl Drop for Types<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.1.free_list)(self.0, Some(self.2));
        }
    }
}
struct AppInfo<'a>(*mut c_void, &'a Gio);
impl Drop for AppInfo<'_> {
    fn drop(&mut self) {
        unsafe {
            (self.1.unref)(self.0);
        }
    }
}

fn extensions(
    snapshot: &mut Snapshot,
    cancel: &Cancellation,
) -> Result<BTreeMap<String, Vec<String>>> {
    let mut directories = dirs::data_local_dir().into_iter().collect::<Vec<_>>();
    let system = std::env::var_os("XDG_DATA_DIRS")
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    directories.extend(std::env::split_paths(&system).filter(|path| path.is_absolute()));
    let mut result: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for directory in directories {
        cancel.check()?;
        let path = directory.join("mime/globs2");
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                super::warning(snapshot, format!("{}: {error}", path.display()));
                continue;
            }
        };
        let mut reader = crate::work::Reader {
            inner: file.take(16 * 1024 * 1024 + 1),
            cancel: cancel.clone(),
        };
        let mut text = String::new();
        if let Err(error) = reader.read_to_string(&mut text) {
            cancel.check()?;
            super::warning(snapshot, format!("{}: {error}", path.display()));
            continue;
        }
        if text.len() > 16 * 1024 * 1024 {
            super::warning(
                snapshot,
                format!("{}: MIME suffix metadata exceeds 16 MiB", path.display()),
            );
            continue;
        }
        super::globs::parse(&text, &mut result, cancel)?;
    }
    for extensions in result.values_mut() {
        extensions.sort();
        extensions.dedup();
    }
    Ok(result)
}

fn api() -> Result<&'static Gio> {
    static API: OnceLock<std::result::Result<Gio, String>> = OnceLock::new();
    API.get_or_init(|| Gio::load().map_err(|error| format!("{error:#}")))
        .as_ref()
        .map_err(|error| anyhow!("{error}"))
}

fn describe(api: &Gio, app: *mut c_void) -> Application {
    let id = string(unsafe { (api.identifier)(app) });
    Application {
        name: string(unsafe { (api.name)(app) })
            .or_else(|| id.clone())
            .unwrap_or_default(),
        identifier: id,
        path: string(unsafe { (api.executable)(app) }).map(PathBuf::from),
    }
}

pub(super) fn offer_type(path: &Path, _: &str) -> Result<Option<(String, Application)>> {
    const DESKTOP_ID: &str = "superfile.desktop";

    let Some(name) = path.file_name() else {
        return Ok(None);
    };
    let api = api()?;
    let name = CString::new(name.as_bytes())?;
    let mut uncertain = 0;
    let guess = unsafe { (api.guess)(name.as_ptr(), std::ptr::null(), 0, &mut uncertain) };
    let mime = string(guess);
    unsafe { (api.free)(guess.cast()) };
    let Some(mime) = mime.filter(|mime| uncertain == 0 && mime != "application/octet-stream")
    else {
        return Ok(None);
    };
    let native_type = CString::new(mime.as_str())?;
    let current = unsafe { (api.default)(native_type.as_ptr(), 0) };
    if !current.is_null() {
        let current = AppInfo(current, api);
        if string(unsafe { (api.identifier)(current.0) }).as_deref() == Some(DESKTOP_ID) {
            return Ok(None);
        }
    }
    let candidate = applications(&mime, &Cancellation::default())?
        .into_iter()
        .find(|app| app.identifier.as_deref() == Some(DESKTOP_ID));
    Ok(candidate.map(|app| (mime, app)))
}

pub(super) fn applications(identifier: &str, cancel: &Cancellation) -> Result<Vec<Application>> {
    let api = api()?;
    let mime = CString::new(identifier)?;
    let apps = Types(unsafe { (api.applications)(mime.as_ptr()) }, api, api.unref);
    let mut node = apps.0;
    let mut result = Vec::new();
    while !node.is_null() {
        cancel.check()?;
        let raw = unsafe { (*node).data };
        node = unsafe { (*node).next };
        if !raw.is_null() {
            result.push(describe(api, raw));
        }
    }
    Ok(result)
}

pub(super) fn set_default(identifier: &str, selected: &Application) -> Result<ChangeResult> {
    let api = api()?;
    let mime = CString::new(identifier)?;
    let apps = Types(unsafe { (api.applications)(mime.as_ptr()) }, api, api.unref);
    let mut node = apps.0;
    while !node.is_null() {
        let raw = unsafe { (*node).data };
        node = unsafe { (*node).next };
        if raw.is_null() || !describe(api, raw).same_handler(selected) {
            continue;
        }
        let mut error = std::ptr::null_mut();
        let success = unsafe { (api.set_default)(raw, mime.as_ptr(), &mut error) };
        let message = if error.is_null() {
            None
        } else {
            let message = string(unsafe { (*error).message });
            unsafe {
                (api.free_error)(error);
            }
            message
        };
        anyhow::ensure!(
            success != 0,
            "{}",
            message
                .as_deref()
                .unwrap_or("GIO rejected the default application change")
        );
        let current = unsafe { (api.default)(mime.as_ptr(), 0) };
        anyhow::ensure!(
            !current.is_null(),
            "GIO did not return a default handler after the change"
        );
        let current = AppInfo(current, api);
        return super::verified(describe(api, current.0), selected);
    }
    anyhow::bail!("The application is no longer registered for this file type")
}

pub(super) fn scan(cancel: &Cancellation) -> Result<Snapshot> {
    let api = api()?;
    let mut snapshot = Snapshot::default();
    let mut extensions = extensions(&mut snapshot, cancel)?;
    let types = Types(unsafe { (api.types)() }, api, api.free);
    let mut current = types.0;
    while !current.is_null() {
        cancel.check()?;
        // Nodes and their strings are owned by `types`, which outlives this iteration.
        let raw = unsafe { (*current).data.cast::<c_char>() };
        current = unsafe { (*current).next };
        let Some(identifier) = string(raw) else {
            continue;
        };
        if identifier.starts_with("x-scheme-handler/") {
            continue;
        }
        let description_ptr = unsafe { (api.description)(raw) };
        let description = string(description_ptr).unwrap_or_default();
        unsafe {
            (api.free)(description_ptr.cast());
        }
        let app = unsafe { (api.default)(raw, 0) };
        let application = if app.is_null() {
            None
        } else {
            let app = AppInfo(app, api);
            let id = string(unsafe { (api.identifier)(app.0) });
            Some(Application {
                name: string(unsafe { (api.name)(app.0) })
                    .or_else(|| id.clone())
                    .unwrap_or_default(),
                identifier: id,
                path: string(unsafe { (api.executable)(app.0) }).map(PathBuf::from),
            })
        };
        snapshot.associations.push(Association {
            extensions: extensions.remove(&identifier).unwrap_or_default(),
            identifier,
            description,
            application,
            issue: None,
        });
    }
    Ok(snapshot)
}
