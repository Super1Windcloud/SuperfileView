//! Application-owned interactions built on the published Markdown plugin seam.
use gpui_kit::{
    base::{
        MarkdownExtensions, MarkdownNode, MarkdownParseContext, MarkdownPlugin, TextView,
        markdown_ast,
    },
    component::{Disableable, checkbox::Checkbox, h_flex, v_flex},
    *,
};
use std::{ops::Range, sync::Arc};

pub(super) type ImageResolver = Arc<dyn Fn(&SharedUri) -> ImageSource + Send + Sync>;

#[derive(Clone)]
struct ImageBlock {
    resolver: ImageResolver,
}
#[derive(Clone)]
struct ImageData {
    uri: String,
    alt: String,
}
impl MarkdownPlugin for ImageBlock {
    fn name(&self) -> &str {
        "superfile-image"
    }
    fn is_block(&self) -> bool {
        true
    }
    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::Paragraph(paragraph) = node else {
            return None;
        };
        let [markdown_ast::Node::Image(image)] = paragraph.children.as_slice() else {
            return None;
        };
        Some(
            MarkdownNode::new(
                self.name(),
                ImageData {
                    uri: image.url.clone(),
                    alt: image.alt.clone(),
                },
            )
            .text(image.alt.clone())
            .markdown(cx.node_source(node).unwrap_or_default().to_owned()),
        )
    }
    fn render(&self, node: &MarkdownNode, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Some(image) = node.data::<ImageData>() else {
            return div().into_any_element();
        };
        let source = (self.resolver)(&SharedUri::from(image.uri.clone()));
        let detail = source.clone();
        div()
            .id((
                "markdown-image",
                node.source_range().map_or(0, |range| range.start),
            ))
            .w_full()
            .h(px(320.))
            .max_h(px(480.))
            .cursor_pointer()
            .role(Role::Image)
            .aria_label(image.alt.clone())
            .on_click(move |_, window, cx| super::image_detail::show(detail.clone(), window, cx))
            .child(img(source).size_full().object_fit(ObjectFit::Contain))
            .into_any_element()
    }
}
#[derive(Clone)]
struct TaskList {
    toggle: Option<super::ToggleTask>,
    editor: EntityId,
    revision: usize,
    nested: MarkdownExtensions,
    resolver: ImageResolver,
}
#[derive(Clone)]
struct Item {
    range: Range<usize>,
    checked: Option<bool>,
    body: String,
}
impl MarkdownPlugin for TaskList {
    fn name(&self) -> &str {
        "superfile-tasks"
    }
    fn is_block(&self) -> bool {
        true
    }
    fn parse(
        &self,
        node: &markdown_ast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        let markdown_ast::Node::List(list) = node else {
            return None;
        };
        if !list
            .children
            .iter()
            .any(|node| matches!(node,markdown_ast::Node::ListItem(item) if item.checked.is_some()))
        {
            return None;
        }
        let items = list
            .children
            .iter()
            .filter_map(|node| {
                let markdown_ast::Node::ListItem(item) = node else {
                    return None;
                };
                let position = item.position.as_ref()?;
                let first = item.children.first()?.position()?;
                let last = item.children.last()?.position()?;
                Some(Item {
                    range: position.start.offset + cx.offset()..position.end.offset + cx.offset(),
                    checked: item.checked,
                    body: cx
                        .source()
                        .get(first.start.offset..last.end.offset)?
                        .to_owned(),
                })
            })
            .collect::<Vec<_>>();
        let source = cx.node_source(node).unwrap_or_default().to_owned();
        Some(
            MarkdownNode::new(self.name(), items)
                .text(source.clone())
                .markdown(source),
        )
    }
    fn render(&self, node: &MarkdownNode, _: &mut Window, _: &mut App) -> impl IntoElement {
        let Some(items) = node.data::<Vec<Item>>() else {
            return div().into_any_element();
        };
        let mut list = v_flex().gap_1().w_full();
        for item in items {
            let mut row = h_flex().w_full().items_start().gap_2();
            if let Some(checked) = item.checked {
                let toggle = self.toggle.clone();
                let range = item.range.clone();
                let editor = self.editor;
                let revision = self.revision;
                row = row.child(
                    Checkbox::new(("markdown-task", range.start))
                        .checked(checked)
                        .accessibility_label(item.body.clone())
                        .disabled(toggle.is_none())
                        .on_click(move |checked, window, cx| {
                            if let Some(toggle) = &toggle {
                                toggle(editor, range.clone(), *checked, revision, window, cx);
                            }
                        }),
                );
            } else {
                row = row.child(div().w(px(16.)).child("•"));
            }
            let resolver = self.resolver.clone();
            row = row.child(
                TextView::markdown(("task-body", item.range.start), item.body.clone())
                    .scrollable(false)
                    .w_full()
                    .markdown_extensions(with_images(self.nested.clone(), self.resolver.clone()))
                    .image_source(move |uri| resolver(uri)),
            );
            list = list.child(row);
        }
        list.into_any_element()
    }
}
pub(super) fn with_images(
    extensions: MarkdownExtensions,
    resolver: ImageResolver,
) -> MarkdownExtensions {
    extensions.plugin(ImageBlock { resolver })
}
pub(super) fn with_tasks(
    extensions: MarkdownExtensions,
    resolver: ImageResolver,
    toggle: Option<super::ToggleTask>,
    editor: EntityId,
    revision: usize,
) -> MarkdownExtensions {
    let nested = extensions.clone();
    with_images(extensions, resolver.clone()).plugin(TaskList {
        toggle,
        editor,
        revision,
        nested,
        resolver,
    })
}
