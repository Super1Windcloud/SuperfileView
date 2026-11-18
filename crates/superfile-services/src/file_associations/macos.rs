use super::*;
use anyhow::{Context, ensure};
use plist_parser::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
};
mod native;

fn application(id: String, cancel: &Cancellation) -> Application {
    let path = native::application_path(&id);
    let info = path
        .as_ref()
        .and_then(|path| read_application_info(path, cancel).ok());
    let name = info
        .as_ref()
        .and_then(Value::as_dictionary)
        .and_then(|info| {
            info.get("CFBundleDisplayName")
                .or_else(|| info.get("CFBundleName"))
        })
        .and_then(Value::as_string)
        .map(str::to_owned)
        .unwrap_or_else(|| id.clone());
    Application {
        name,
        identifier: Some(id),
        path,
    }
}

pub(super) fn applications(identifier: &str, cancel: &Cancellation) -> Result<Vec<Application>> {
    let mut applications = Vec::new();
    for id in native::handlers(identifier) {
        cancel.check()?;
        let app = application(id, cancel);
        if app.path.as_ref().is_some_and(|path| path.is_dir()) {
            applications.push(app);
        }
    }
    Ok(applications)
}

pub(super) fn set_default(identifier: &str, selected: &Application) -> Result<ChangeResult> {
    let id = selected
        .identifier
        .as_deref()
        .context("Application bundle identifier is missing")?;
    if native::handler(identifier).as_deref() != Some(id) {
        native::set_handler(identifier, id)?;
    }
    let id = native::handler(identifier)
        .context("macOS did not return a default handler after the change")?;
    super::verified(application(id, &Cancellation::default()), selected)
}

#[derive(Default)]
struct Declaration {
    description: String,
    extensions: BTreeSet<String>,
}
#[derive(Default)]
struct Types {
    declarations: BTreeMap<String, Declaration>,
    extensions: BTreeSet<String>,
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(value)) => vec![value.clone()],
        Some(Value::Array(values)) => values
            .iter()
            .filter_map(Value::as_string)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

impl Types {
    fn read(&mut self, value: &Value) {
        let Some(info) = value.as_dictionary() else {
            return;
        };
        for key in ["UTExportedTypeDeclarations", "UTImportedTypeDeclarations"] {
            for value in info
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(declaration) = value.as_dictionary() else {
                    continue;
                };
                let Some(identifier) = declaration
                    .get("UTTypeIdentifier")
                    .and_then(Value::as_string)
                else {
                    continue;
                };
                let entry = self.declarations.entry(identifier.to_owned()).or_default();
                if let Some(description) = declaration
                    .get("UTTypeDescription")
                    .and_then(Value::as_string)
                {
                    entry.description = description.to_owned();
                }
                let tags = declaration
                    .get("UTTypeTagSpecification")
                    .and_then(Value::as_dictionary);
                for extension in
                    strings(tags.and_then(|tags| tags.get("public.filename-extension")))
                {
                    if extension != "*" && !extension.is_empty() {
                        entry.extensions.insert(extension);
                    }
                }
            }
        }
        for value in info
            .get("CFBundleDocumentTypes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(document) = value.as_dictionary() else {
                continue;
            };
            for identifier in strings(document.get("LSItemContentTypes")) {
                self.declarations.entry(identifier).or_default();
            }
            for extension in strings(document.get("CFBundleTypeExtensions")) {
                if extension != "*" && !extension.is_empty() {
                    self.extensions.insert(extension);
                }
            }
        }
        for value in info
            .get("LSHandlers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(handler) = value.as_dictionary() else {
                continue;
            };
            if let Some(identifier) = handler
                .get("LSHandlerContentType")
                .and_then(Value::as_string)
            {
                self.declarations.entry(identifier.to_owned()).or_default();
            }
            if handler
                .get("LSHandlerContentTagClass")
                .and_then(Value::as_string)
                == Some("public.filename-extension")
            {
                self.extensions
                    .extend(strings(handler.get("LSHandlerContentTag")));
            }
        }
    }
}

fn read_info(path: &Path, cancel: &Cancellation) -> Result<Value> {
    let file = std::fs::File::open(path)?;
    let mut reader = crate::work::Reader {
        inner: file.take(16 * 1024 * 1024 + 1),
        cancel: cancel.clone(),
    };
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "Application metadata exceeds 16 MiB"
    );
    Value::from_reader(std::io::Cursor::new(bytes))
        .with_context(|| format!("Cannot read {}", path.display()))
}

fn read_application_info(bundle: &Path, cancel: &Cancellation) -> Result<Value> {
    for relative in [
        "Contents/Info.plist",
        "WrappedBundle/Info.plist",
        "Info.plist",
    ] {
        cancel.check()?;
        let path = bundle.join(relative);
        match read_info(&path, cancel) {
            Err(error)
                if error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
            result => return result,
        }
    }
    anyhow::bail!("No Info.plist found in application bundle");
}

fn discover_applications(
    root: PathBuf,
    paths: &mut BTreeSet<PathBuf>,
    snapshot: &mut Snapshot,
    cancel: &Cancellation,
) -> Result<()> {
    let mut pending = vec![root];
    while let Some(directory) = pending.pop() {
        cancel.check()?;
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                super::warning(snapshot, format!("{}: {error}", directory.display()));
                continue;
            }
        };
        for entry in entries {
            cancel.check()?;
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    super::warning(snapshot, error.to_string());
                    continue;
                }
            };
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "app") {
                paths.insert(path);
            } else if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                pending.push(path);
            }
        }
    }
    Ok(())
}

pub(super) fn scan(cancel: &Cancellation) -> Result<Snapshot> {
    let mut snapshot = Snapshot::default();
    let mut types = Types::default();
    let home = crate::platform::home()?;
    let core_types = Path::new("/System/Library/CoreServices/CoreTypes.bundle/Contents/Info.plist");
    match read_info(core_types, cancel) {
        Ok(info) => types.read(&info),
        Err(error) => {
            cancel.check()?;
            super::warning(&mut snapshot, error.to_string());
        }
    }
    let mut paths = BTreeSet::new();
    match native::indexed_applications(cancel) {
        Ok(indexed) => paths.extend(indexed),
        Err(error) => {
            cancel.check()?;
            super::warning(&mut snapshot, error.to_string());
        }
    }
    for root in [
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Library/CoreServices/Applications"),
        home.join("Applications"),
    ] {
        discover_applications(root, &mut paths, &mut snapshot, cancel)?;
    }
    for application in paths {
        cancel.check()?;
        match read_application_info(&application, cancel) {
            Ok(info) => types.read(&info),
            Err(error) => {
                cancel.check()?;
                super::warning(&mut snapshot, format!("{}: {error}", application.display()));
            }
        }
    }
    let overrides = home
        .join("Library/Preferences/com.apple.LaunchServices/com.apple.launchservices.secure.plist");
    match read_info(&overrides, cancel) {
        Ok(info) => types.read(&info),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) => {}
        Err(error) => {
            cancel.check()?;
            super::warning(&mut snapshot, error.to_string());
        }
    }
    for extension in types.extensions {
        cancel.check()?;
        for identifier in native::identifiers(&extension) {
            types
                .declarations
                .entry(identifier)
                .or_default()
                .extensions
                .insert(extension.clone());
        }
    }
    let mut applications = BTreeMap::new();
    for (identifier, mut declaration) in types.declarations {
        cancel.check()?;
        let (description, extensions) = native::details(&identifier);
        declaration.extensions.extend(extensions);
        let application = native::handler(&identifier).map(|id| {
            applications
                .entry(id.clone())
                .or_insert_with(|| application(id, cancel))
                .clone()
        });
        snapshot.associations.push(Association {
            identifier,
            description: description.unwrap_or(declaration.description),
            extensions: declaration.extensions.into_iter().collect(),
            application,
            issue: None,
        });
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_wrapped_and_flat_application_metadata_with_native_layout_precedence() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let bundle = directory.path().join("Example.app");
        let wrapped = bundle.join("Wrapper/Runner.app");
        std::fs::create_dir_all(&wrapped)?;
        std::os::unix::fs::symlink("Wrapper/Runner.app", bundle.join("WrappedBundle"))?;
        let cancel = Cancellation::default();
        assert!(read_application_info(&bundle, &cancel).is_err());

        let mut metadata = plist_parser::Dictionary::new();
        metadata.insert(
            "CFBundleDisplayName".into(),
            Value::String("Wrapped".into()),
        );
        let wrapped_info = Value::Dictionary(metadata.clone());
        wrapped_info.to_file_xml(wrapped.join("Info.plist"))?;
        assert_eq!(read_application_info(&wrapped, &cancel)?, wrapped_info);
        assert_eq!(read_application_info(&bundle, &cancel)?, wrapped_info);

        let contents = bundle.join("Contents");
        std::fs::create_dir(&contents)?;
        metadata.insert("CFBundleDisplayName".into(), Value::String("Native".into()));
        let native_info = Value::Dictionary(metadata);
        native_info.to_file_xml(contents.join("Info.plist"))?;
        assert_eq!(read_application_info(&bundle, &cancel)?, native_info);
        std::fs::write(contents.join("Info.plist"), "malformed plist")?;
        assert!(read_application_info(&bundle, &cancel).is_err());
        Ok(())
    }

    #[test]
    fn system_and_application_declarations_include_legacy_types_and_user_overrides() {
        let xml = br#"<?xml version="1.0"?><plist version="1.0"><dict>
        <key>UTExportedTypeDeclarations</key><array><dict><key>UTTypeIdentifier</key><string>example.type</string><key>UTTypeTagSpecification</key><dict><key>public.filename-extension</key><array><string>foo</string><string>bar</string></array></dict></dict></array>
        <key>CFBundleDocumentTypes</key><array><dict><key>LSItemContentTypes</key><array><string>public.pdf</string></array><key>CFBundleTypeExtensions</key><array><string>custom</string><string>*</string></array></dict></array>
        <key>LSHandlers</key><array><dict><key>LSHandlerContentType</key><string>user.custom</string></dict><dict><key>LSHandlerContentTagClass</key><string>public.filename-extension</string><key>LSHandlerContentTag</key><string>mine</string></dict></array>
        </dict></plist>"#;
        let value = Value::from_reader_xml(&xml[..]).unwrap();
        let mut types = Types::default();
        types.read(&value);
        assert!(types.declarations.contains_key("public.pdf"));
        assert!(types.declarations.contains_key("user.custom"));
        assert_eq!(
            types.declarations["example.type"]
                .extensions
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec!["bar", "foo"]
        );
        assert_eq!(
            types.extensions.into_iter().collect::<Vec<_>>(),
            vec!["custom", "mine"]
        );
    }
}
