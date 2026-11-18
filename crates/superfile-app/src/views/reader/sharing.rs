use super::{Reader, Renderer};
use anyhow::{Context, Result};
use gpui_kit::{App, component::input::Rope};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};
use superfile_core::session::SessionTab;
use superfile_services::{paged::PageEdit, remote::Session, sharing::Contents};

pub(crate) struct ShareSource {
    pub name: OsString,
    pub directory: Option<PathBuf>,
    pub original: Option<PathBuf>,
    body: Body,
}

enum Body {
    Disk(PathBuf, Option<Session>),
    Text(Rope, Option<PageEdit>),
}

impl ShareSource {
    pub fn prepare(
        self,
        cancel: &superfile_services::work::Cancellation,
    ) -> Result<superfile_services::sharing::SharedFile> {
        let (contents, _session) = self.body.materialize();
        contents.prepare(&self.name, cancel)
    }

    pub fn save_copy(
        self,
        path: &Path,
        mut open: Vec<PathBuf>,
        cancel: &superfile_services::work::Cancellation,
    ) -> Result<PathBuf> {
        open.extend(self.original);
        let (contents, _session) = self.body.materialize();
        contents.save_copy(path, &open, cancel)
    }
}

impl Body {
    fn materialize(self) -> (Contents, Option<Session>) {
        match self {
            Self::Disk(path, Some(session)) => (Contents::Cached(path), Some(session)),
            Self::Disk(path, None) => (Contents::Local(path), None),
            Self::Text(text, Some(page)) => (Contents::EditedPage(page, text.to_string()), None),
            Self::Text(text, None) => (Contents::Text(text.to_string()), None),
        }
    }
}

impl Reader {
    pub(crate) fn can_share_file(&self, cx: &App) -> bool {
        !self.is_unavailable() && matches!(self.session_tab(cx), SessionTab::File { .. })
    }

    pub(crate) fn share_source(&self, cx: &App) -> Result<ShareSource> {
        let path = &self.document.path;
        let draft = self.document.draft_directory.as_ref();
        let remote = self.document.remote.clone();
        let name = if draft.is_some() {
            let title = Path::new(self.title.as_ref());
            if title.extension().is_some() {
                self.title.as_ref().into()
            } else {
                format!("{}.txt", self.title).into()
            }
        } else {
            path.file_name().context("The file has no name")?.to_owned()
        };
        let original = (draft.is_none() && remote.is_none()).then(|| path.clone());
        let directory = draft
            .cloned()
            .or_else(|| original.as_ref()?.parent().map(Path::to_path_buf));
        let body = if draft.is_some() || self.is_dirty() {
            let editor = self
                .editor()
                .context("The editable document is unavailable")?;
            let page = if let Renderer::Paged { view, .. } = &self.renderer {
                Some(
                    view.read(cx)
                        .edit_snapshot()
                        .context("The editable page is unavailable")?,
                )
            } else {
                None
            };
            Body::Text(editor.read(cx).text().clone(), page)
        } else {
            let source = match &remote {
                Some(session) => session.cache_path(path)?,
                None => path.clone(),
            };
            Body::Disk(source, remote)
        };
        Ok(ShareSource {
            name,
            directory,
            original,
            body,
        })
    }
}
