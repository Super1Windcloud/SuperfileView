use gpui_kit::{
    base::{TextView, TextViewState, TextViewStyle},
    component::{ActiveTheme, h_flex, scroll::Scrollbar, v_flex},
    *,
};
use std::{fmt::Write as _, sync::Arc};
use superfile_services::rich_documents::docx::{
    WordDocument,
    model::{Alignment, Block, CellContent, ImageBlock, Inline, Paragraph, Table},
};

pub(super) struct WordView {
    blocks: Arc<[Row]>,
    search: super::super::find::PreviewGroup,
    _memory: superfile_services::resources::WorkingReservation,
}

enum Row {
    Content(Content),
    Table(TableRow),
}

enum Content {
    Text {
        state: Entity<TextViewState>,
        alignment: Alignment,
        font_size: u16,
        font_family: SharedString,
        color: Option<u32>,
        highlight: Option<u32>,
        spacing: (u16, u16, u16, u16),
    },
    Image {
        image: Arc<Image>,
        size: Size<Pixels>,
        alignment: Alignment,
    },
    Table(Vec<TableRow>),
    Rule,
}

struct TableRow {
    indent: f32,
    cells: Vec<Cell>,
}

struct Cell {
    width: f32,
    fill: Option<u32>,
    contents: Vec<Content>,
}

impl WordView {
    pub(super) fn new(document: WordDocument, cx: &mut App) -> Self {
        let mut blocks = Vec::new();
        let mut paragraphs = Vec::new();
        append_blocks(document.blocks, 0, &mut blocks, &mut paragraphs, cx);
        let list = ListState::new(blocks.len(), ListAlignment::Top, px(256.));
        Self {
            blocks: blocks.into(),
            search: super::super::find::PreviewGroup {
                paragraphs: paragraphs.into(),
                list,
                focus: cx.focus_handle(),
            },
            _memory: document.memory,
        }
    }

    pub(super) fn search_target(&self) -> super::super::find::Target {
        super::super::find::Target::Group(self.search.clone())
    }

    pub(super) fn render(&self, cx: &App) -> impl IntoElement {
        let blocks = self.blocks.clone();
        div()
            .id("word-continuous")
            .track_focus(&self.search.focus)
            .size_full()
            .relative()
            .bg(super::super::editor_paper(cx))
            .child(
                list(self.search.list.clone(), move |index, _, cx| {
                    render_row(&blocks[index], cx).into_any_element()
                })
                .size_full(),
            )
            .child(Scrollbar::vertical(&self.search.list))
    }
}

fn append_blocks(
    source: Vec<Block>,
    depth: usize,
    rows: &mut Vec<Row>,
    paragraphs: &mut Vec<(usize, Entity<TextViewState>)>,
    cx: &mut App,
) {
    for block in source {
        let item = rows.len();
        match block {
            Block::Paragraph(value) => {
                rows.push(Row::Content(paragraph(&value, None, item, paragraphs, cx)));
            }
            Block::Heading(value) => rows.push(Row::Content(paragraph(
                &value.content,
                Some((value.level, String::new())),
                item,
                paragraphs,
                cx,
            ))),
            Block::Table(value) => rows.extend(
                table_rows(&value, rows.len(), true, paragraphs, cx)
                    .into_iter()
                    .map(Row::Table),
            ),
            Block::Image(value) => {
                rows.push(Row::Content(figure(
                    &value,
                    Alignment::Start,
                    item,
                    paragraphs,
                    cx,
                )));
            }
            Block::List(value) => {
                for (index, entry) in value.items.into_iter().enumerate() {
                    let bullet = if value.ordered {
                        format!("{}. ", index + 1)
                    } else {
                        "• ".to_owned()
                    };
                    let mut children = entry.blocks.into_iter();
                    if let Some(first) = children.next() {
                        match first {
                            Block::Paragraph(mut value) => {
                                value.style.margin_left_mm = value
                                    .style
                                    .margin_left_mm
                                    .max((depth.min(10) as u16 + 1) * 4);
                                rows.push(Row::Content(paragraph(
                                    &value,
                                    Some((0, bullet)),
                                    rows.len(),
                                    paragraphs,
                                    cx,
                                )));
                            }
                            other => append_blocks(vec![other], depth + 1, rows, paragraphs, cx),
                        }
                        append_blocks(children.collect(), depth + 1, rows, paragraphs, cx);
                    }
                }
            }
            Block::Section(value) => append_blocks(value.blocks, depth, rows, paragraphs, cx),
            Block::HorizontalRule => rows.push(Row::Content(Content::Rule)),
            Block::PageBreak => {}
        }
    }
}

fn table_rows(
    table: &Table,
    first_item: usize,
    virtual_rows: bool,
    paragraphs: &mut Vec<(usize, Entity<TextViewState>)>,
    cx: &mut App,
) -> Vec<TableRow> {
    let grid = table.docx_grid.as_deref().unwrap_or_default();
    let columns = table.rows.iter().fold(grid.len(), |columns, row| {
        columns.max(
            usize::from(row.grid_before)
                + row
                    .cells
                    .iter()
                    .map(|cell| usize::from(cell.grid_span.max(1)))
                    .sum::<usize>(),
        )
    });
    let mut widths = Vec::with_capacity(columns + 1);
    widths.push(0.);
    for column in 0..columns {
        let width = f64::from(
            grid.get(column)
                .copied()
                .filter(|w| *w != 0)
                .unwrap_or(1440),
        );
        widths.push(widths[column] + width);
    }
    let total = widths[columns].max(1.);
    let mut rows = Vec::with_capacity(table.rows.len());
    for (offset, row) in table.rows.iter().enumerate() {
        let item = first_item + if virtual_rows { offset } else { 0 };
        let mut column = usize::from(row.grid_before);
        let indent = (widths[column] / total) as f32;
        let mut cells = Vec::with_capacity(row.cells.len());
        for cell in &row.cells {
            let next = column + usize::from(cell.grid_span.max(1));
            let width = ((widths[next] - widths[column]) / total) as f32;
            column = next;
            let contents = cell
                .contents()
                .map(|content| match content {
                    CellContent::Paragraph(value) => paragraph(value, None, item, paragraphs, cx),
                    CellContent::Image(value) => {
                        figure(&value.image, value.alignment.clone(), item, paragraphs, cx)
                    }
                    CellContent::Table(value) => {
                        Content::Table(table_rows(value, item, false, paragraphs, cx))
                    }
                })
                .collect();
            cells.push(Cell {
                width,
                fill: cell.fill.as_deref().and_then(rgb_value),
                contents,
            });
        }
        rows.push(TableRow { indent, cells });
    }
    rows
}

fn rgb_value(value: &str) -> Option<u32> {
    let value = value.strip_prefix('#').unwrap_or(value);
    (value.len() == 6)
        .then(|| u32::from_str_radix(value, 16).ok())
        .flatten()
}

fn escape_text(output: &mut String, text: &str) {
    // Numeric entities keep literal DOCX punctuation from becoming Markdown syntax.
    for character in text.chars() {
        match character {
            '\n' => output.push_str("<br/>"),
            '\t' => output.push_str("&#160;&#160;&#160;&#160;"),
            ' ' => output.push_str("&#32;"),
            character if character.is_ascii_punctuation() => {
                let _ = write!(output, "&#{};", u32::from(character));
            }
            character => output.push(character),
        }
    }
}

fn paragraph(
    value: &Paragraph,
    lead: Option<(u8, String)>,
    item: usize,
    paragraphs: &mut Vec<(usize, Entity<TextViewState>)>,
    cx: &mut App,
) -> Content {
    let (heading, prefix) = lead.unwrap_or_default();
    let mut source = String::new();
    escape_text(&mut source, &prefix);
    let mut leading_style = None;
    for span in &value.spans {
        let (text, link, bold, italic, underline, code) = match span {
            Inline::Text(text) => (text, None, heading > 0, false, false, false),
            Inline::Link { label, url } => (label, Some(url), heading > 0, false, false, false),
            Inline::Bold(text) => (text, None, true, false, false, false),
            Inline::Italic(text) => (text, None, heading > 0, true, false, false),
            Inline::Code(text) => (text, None, heading > 0, false, false, true),
            Inline::Styled { text, style, url } => {
                if !text.trim().is_empty() {
                    leading_style.get_or_insert(style);
                }
                (
                    text,
                    url.as_ref(),
                    style.bold || heading > 0,
                    style.italic,
                    style.underline,
                    false,
                )
            }
            Inline::LineBreak => {
                source.push_str("<br/>");
                continue;
            }
        };
        let link = link
            .and_then(|link| url::Url::parse(link).ok())
            .filter(|link| matches!(link.scheme(), "http" | "https"));
        if link.is_some() {
            source.push('[');
        }
        let tags = [
            ("strong", bold),
            ("em", italic),
            ("u", underline),
            ("code", code),
        ];
        for (tag, enabled) in tags {
            if enabled {
                let _ = write!(source, "<{tag}>");
            }
        }
        escape_text(&mut source, text);
        for (tag, enabled) in tags.into_iter().rev() {
            if enabled {
                let _ = write!(source, "</{tag}>");
            }
        }
        if let Some(link) = link {
            let _ = write!(source, "](<{link}>)");
        }
    }
    let style = leading_style.unwrap_or(&value.text_style);
    let font_size = match (style.font_size_pt, heading) {
        (size, _) if size != 0 => size,
        (_, 1) => 20,
        (_, 2) => 16,
        (_, 3..=u8::MAX) => 14,
        _ => 11,
    };
    let state = cx.new(|cx| TextViewState::markdown(&source, cx));
    paragraphs.push((item, state.clone()));
    Content::Text {
        state,
        alignment: value.style.alignment.clone(),
        font_size,
        font_family: style.font_family.clone().into(),
        color: rgb_value(&style.color),
        highlight: rgb_value(&style.background),
        spacing: (
            value.style.margin_top_mm,
            value.style.margin_bottom_mm,
            value.style.margin_left_mm,
            value.style.margin_right_mm,
        ),
    }
}

fn figure(
    value: &ImageBlock,
    alignment: Alignment,
    item: usize,
    paragraphs: &mut Vec<(usize, Entity<TextViewState>)>,
    cx: &mut App,
) -> Content {
    let format = match value.mime_type.as_str() {
        "image/png" => Some(ImageFormat::Png),
        "image/jpeg" => Some(ImageFormat::Jpeg),
        "image/gif" => Some(ImageFormat::Gif),
        "image/webp" => Some(ImageFormat::Webp),
        "image/bmp" => Some(ImageFormat::Bmp),
        "image/tiff" => Some(ImageFormat::Tiff),
        "image/svg+xml" => Some(ImageFormat::Svg),
        _ => None,
    };
    if let Some(format) = format.filter(|_| !value.data.is_empty()) {
        return Content::Image {
            image: Arc::new(Image::from_bytes(format, value.data.clone())),
            size: size(
                px((value.size.width.as_pt() * 4. / 3.).max(1.)),
                px((value.size.height.as_pt() * 4. / 3.).max(1.)),
            ),
            alignment,
        };
    }
    let mut placeholder = Paragraph::plain(if value.alt.is_empty() {
        superfile_services::locale::tr("Image unavailable").to_string()
    } else {
        value.alt.clone()
    });
    placeholder.style.alignment = alignment;
    paragraph(&placeholder, None, item, paragraphs, cx)
}

fn render_row(row: &Row, cx: &App) -> Div {
    let base = px(16.);
    let scale = f32::from(crate::app::text_zoom::font_size(base, cx)) / f32::from(base);
    let wrap = div().w_full().px_6();
    match row {
        Row::Content(content) => wrap.py_1().child(render_content(content, scale, None, cx)),
        Row::Table(row) => wrap.child(render_table_row(row, scale, cx)),
    }
}

fn render_table_row(row: &TableRow, scale: f32, cx: &App) -> Div {
    h_flex()
        .w_full()
        .items_stretch()
        .child(div().w(relative(row.indent)).flex_none())
        .children(row.cells.iter().map(|cell| {
            let fill = cell.fill.map(|color| Hsla::from(rgb(color)));
            let mut element = v_flex()
                .w(relative(cell.width))
                .min_w_0()
                .flex_none()
                .border_1()
                .border_color(cx.theme().border)
                .px_2()
                .py_1()
                .children(
                    cell.contents
                        .iter()
                        .map(|c| render_content(c, scale, fill, cx)),
                );
            if let Some(fill) = fill {
                element = element.bg(fill);
            }
            element
        }))
}

fn render_content(content: &Content, scale: f32, fill: Option<Hsla>, cx: &App) -> AnyElement {
    match content {
        Content::Text {
            state,
            alignment,
            font_size,
            font_family,
            color,
            highlight,
            spacing,
        } => {
            let paper = fill.unwrap_or_else(|| super::super::editor_paper(cx));
            let ink = color
                .map(|color| Hsla::from(rgb(color)))
                .filter(|ink| (paper.l - ink.l).abs() >= 0.42)
                .unwrap_or_else(|| {
                    if fill.is_some() {
                        if paper.l > 0.55 { black() } else { white() }
                    } else {
                        super::super::editor_ink(cx)
                    }
                });
            let mut text = TextView::new(state)
                .w_full()
                .selectable(true)
                .text_size(px(f32::from(*font_size) * 4. / 3. * scale))
                .text_align(match alignment {
                    Alignment::Start | Alignment::Justify => TextAlign::Left,
                    Alignment::Center => TextAlign::Center,
                    Alignment::End => TextAlign::Right,
                })
                .style(
                    TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx))
                        .with_foreground(ink)
                        .with_paragraph_gap(rems(0.)),
                )
                .on_link_click(|uri, _, _, cx| {
                    if uri.starts_with("https://") || uri.starts_with("http://") {
                        cx.open_url(uri);
                    }
                });
            if !font_family.is_empty() {
                text = text.font_family(font_family.clone());
            }
            let mut block = div()
                .min_h(px(f32::from(*font_size) * scale))
                .pt(px(f32::from(spacing.0) * 96. / 25.4 * scale))
                .pb(px(f32::from(spacing.1) * 96. / 25.4 * scale))
                .pl(px(f32::from(spacing.2) * 96. / 25.4 * scale))
                .pr(px(f32::from(spacing.3) * 96. / 25.4 * scale))
                .child(text);
            if let Some(highlight) = highlight {
                block = block.bg(paper.blend(Hsla::from(rgb(*highlight)).opacity(0.35)));
            }
            block.into_any_element()
        }
        Content::Image {
            image,
            size,
            alignment,
        } => {
            let mut line = h_flex().w_full();
            line = match alignment {
                Alignment::Start | Alignment::Justify => line.justify_start(),
                Alignment::Center => line.justify_center(),
                Alignment::End => line.justify_end(),
            };
            line.child(
                img(image.clone())
                    .w(size.width * scale)
                    .max_w_full()
                    .aspect_ratio(f32::from(size.width) / f32::from(size.height))
                    .object_fit(ObjectFit::Contain),
            )
            .into_any_element()
        }
        Content::Table(rows) => v_flex()
            .w_full()
            .children(rows.iter().map(|row| render_table_row(row, scale, cx)))
            .into_any_element(),
        Content::Rule => div()
            .w_full()
            .h(px(1.))
            .bg(cx.theme().border)
            .into_any_element(),
    }
}
