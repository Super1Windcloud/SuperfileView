//! File types, applications, and the operating system's default handlers.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Application {
    pub name: String,
    pub identifier: Option<String>,
    pub path: Option<PathBuf>,
}

impl Application {
    pub fn same_handler(&self, other: &Self) -> bool {
        if let (Some(left), Some(right)) = (&self.identifier, &other.identifier) {
            if left == right {
                return true;
            }
            #[cfg(not(windows))]
            return false;
        }
        match (&self.path, &other.path) {
            (Some(left), Some(right)) => {
                #[cfg(windows)]
                {
                    left.as_os_str()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
                }
                #[cfg(not(windows))]
                {
                    left == right
                }
            }
            _ => false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Association {
    pub identifier: String,
    pub extensions: Vec<String>,
    pub description: String,
    pub application: Option<Application>,
    pub issue: Option<String>,
}

impl Association {
    pub fn label(&self) -> String {
        if self.extensions.is_empty() {
            self.identifier.clone()
        } else {
            self.extensions
                .iter()
                .map(|ext| format!(".{ext}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    pub fn matches(&self, query: &str) -> bool {
        let mut text = format!(
            "{} {} {}",
            self.identifier,
            self.extensions.join(" "),
            self.description
        );
        if let Some(app) = &self.application {
            text.push_str(&format!(
                " {} {} {}",
                app.name,
                app.identifier.as_deref().unwrap_or_default(),
                app.path
                    .as_ref()
                    .map(|path| path.to_string_lossy())
                    .unwrap_or_default()
            ));
        }
        let text = text.to_lowercase();
        query
            .split_whitespace()
            .all(|word| text.contains(&word.to_lowercase()))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub associations: Vec<Association>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_combines_extension_type_and_application_without_hiding_unassigned_types() {
        let mut entry = Association {
            identifier: "public.pdf".into(),
            extensions: vec!["pdf".into()],
            description: "PDF 文档".into(),
            application: Some(Application {
                name: "Preview".into(),
                identifier: Some("com.apple.Preview".into()),
                path: Some("/System/Applications/Preview.app".into()),
            }),
            issue: None,
        };
        assert!(entry.matches(" PDF  preview "));
        assert!(entry.matches("文档 apple"));
        assert!(!entry.matches("pdf chrome"));
        assert_eq!(entry.label(), ".pdf");
        entry.application = None;
        assert!(entry.matches("pdf"));
        assert!(!entry.matches("preview"));
        entry.extensions.clear();
        assert_eq!(entry.label(), "public.pdf");
    }
}
