use gpui_kit::{
    base::{TextView, TextViewState, TextViewStyle},
    component::{ActiveTheme, h_flex, scroll::Scrollbar, v_flex},
    prelude::FluentBuilder,
    *,
};
use std::{cell::RefCell, fmt::Write, rc::Rc, sync::Arc};
use superfile_services::rich_documents::docx::{WordDocument, ir};

use super::super::{editor_ink, editor_paper, find, media::PreparedImage};

pub(super) struct WordView {
    blocks: Rc<[Block]>,
    paragraphs: SearchParagraphs,
    list: ListState,
    focus: FocusHandle,
    partial: bool,
    _memory: superfile_services::resources::WorkingReservation,
}

enum Block {
    Text(Text),
    Table(Table),
    Image(Entity<PreparedImage>, Size<Pixels>),
    Group(Vec<Block>),
    Rule,
}

struct Text {
    state: Rc<RefCell<ParagraphState>>,
    size: f32,
    font: Option<SharedString>,
    color: Option<[u8; 3]>,
    fill: Option<[u8; 3]>,
    marked_background: Option<[u8; 3]>,
    alignment: Option<ir::ParagraphAlignment>,
    spacing: [f32; 4],
}

enum ParagraphState {
    Pending(String),
    Ready(Entity<TextViewState>),
}

fn text_entity(state: &RefCell<ParagraphState>, cx: &mut App) -> Entity<TextViewState> {
    let mut state = state.borrow_mut();
    match &*state {
        ParagraphState::Ready(entity) => entity.clone(),
        ParagraphState::Pending(html) => {
            let entity = cx.new(|cx| TextViewState::html(html, cx));
            *state = ParagraphState::Ready(entity.clone());
            entity
        }
    }
}

const TABLE_ROWS_PER_BLOCK: usize = 32;

struct Table {
    widths: Arc<[f32]>,
    heights: Vec<f32>,
    cells: Vec<Cell>,
    scroll: ScrollHandle,
    first: bool,
    last: bool,
}

struct Cell {
    column: usize,
    row: usize,
    columns: usize,
    rows: usize,
    fill: Option<[u8; 3]>,
    vertical_align: Option<ir::CellVerticalAlign>,
    padding: [f32; 4],
    blocks: Vec<Block>,
    continues: bool,
}

impl WordView {
    pub(super) fn new(document: WordDocument, cx: &mut App) -> Self {
        let mut blocks = Vec::new();
        let mut paragraphs = Vec::new();
        for mut element in document.blocks {
            if let ir::Element::Table(table) = &mut element {
                let start = blocks.len();
                let table = build_table(
                    table,
                    |row| start + row / TABLE_ROWS_PER_BLOCK,
                    &mut paragraphs,
                    cx,
                );
                blocks.extend(table.into_bands().into_iter().map(Block::Table));
            } else if let Some(block) = build_block(&mut element, blocks.len(), &mut paragraphs, cx)
            {
                blocks.push(block);
            }
        }
        let list = ListState::new(blocks.len(), ListAlignment::Top, px(96.));
        Self {
            blocks: blocks.into(),
            paragraphs,
            list,
            focus: cx.focus_handle(),
            partial: document.partial,
            _memory: document.memory,
        }
    }

    pub(super) fn search_target(&self, cx: &mut App) -> find::Target {
        find::Target::Group(find::PreviewGroup {
            paragraphs: self
                .paragraphs
                .iter()
                .map(|(item, state)| (*item, text_entity(state, cx)))
                .collect(),
            list: self.list.clone(),
            focus: self.focus.clone(),
        })
    }

    pub(super) fn suspend(&self, cx: &mut App) {
        for block in self.blocks.iter() {
            block.suspend(cx);
        }
    }

    pub(super) fn render(&self, cx: &App) -> impl IntoElement {
        let blocks = self.blocks.clone();
        v_flex()
            .id("word-continuous")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .bg(editor_paper(cx))
            .when(self.partial, |view| {
                view.child(
                    div()
                        .px_6()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(superfile_services::locale::tr(
                            "Some Word content could not be read. This preview is incomplete.",
                        )),
                )
            })
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        list(self.list.clone(), move |index, _, cx| {
                            let scale =
                                f32::from(crate::app::text_zoom::font_size(px(16.), cx)) / 16.;
                            let paper = editor_paper(cx);
                            let block = &blocks[index];
                            let (top, bottom) = match block {
                                Block::Table(table) => (table.first, table.last),
                                _ => (true, true),
                            };
                            div()
                                .id(("word-block", index))
                                .w_full()
                                .px_6()
                                .when(top, |block| block.pt_1())
                                .when(bottom, |block| block.pb_1())
                                .child(block.render(scale, paper, cx))
                                .into_any_element()
                        })
                        .size_full(),
                    )
                    .child(Scrollbar::vertical(&self.list)),
            )
    }
}

type SearchParagraphs = Vec<(usize, Rc<RefCell<ParagraphState>>)>;

fn build_blocks(
    elements: &mut [ir::Element],
    item: usize,
    paragraphs: &mut SearchParagraphs,
    cx: &mut App,
) -> Vec<Block> {
    elements
        .iter_mut()
        .filter_map(|e| build_block(e, item, paragraphs, cx))
        .collect()
}

fn build_block(
    element: &mut ir::Element,
    item: usize,
    paragraphs: &mut SearchParagraphs,
    cx: &mut App,
) -> Option<Block> {
    use ir::Element::*;
    let block = match element {
        Paragraph(p) => Block::Text(text(p, None, item, paragraphs)),
        Heading(h) => {
            let level = h.clamped_level();
            let p = ir::Paragraph {
                content: std::mem::take(&mut h.content),
                alignment: h.alignment.clone(),
                space_before_twips: h.space_before_twips,
                space_after_twips: h.space_after_twips,
                indent_left_twips: h.indent_left_twips,
                indent_right_twips: h.indent_right_twips,
                background_color: h.background_color,
                ..Default::default()
            };
            Block::Text(text(&p, Some(level), item, paragraphs))
        }
        Table(table) => Block::Table(build_table(table, |_| item, paragraphs, cx)),
        Image(image) => {
            let Some(bytes) = image.data.take() else {
                let label = image
                    .alt_text
                    .clone()
                    .filter(|label| !label.is_empty())
                    .unwrap_or_else(|| superfile_services::locale::tr("Image unavailable"));
                let p = ir::Paragraph {
                    content: vec![ir::InlineContent::Text(ir::TextSpan::plain(label))],
                    ..Default::default()
                };
                return Some(Block::Text(text(&p, None, item, paragraphs)));
            };
            let extent = |display: Option<u64>, intrinsic: Option<u32>, fallback| {
                display
                    .filter(|&n| n > 0)
                    .map(|n| n as f32 / 9525.)
                    .or(intrinsic.map(|n| n as f32))
                    .unwrap_or(fallback)
                    .clamp(1., 4096.)
            };
            Block::Image(
                cx.new(|cx| PreparedImage::embedded(bytes, cx)),
                size(
                    px(extent(image.display_width_emu, image.pixel_width, 300.)),
                    px(extent(image.display_height_emu, image.pixel_height, 200.)),
                ),
            )
        }
        List(list) => Block::Group(build_list(list, item, paragraphs, cx)),
        TextBox(boxed) => Block::Group(build_blocks(&mut boxed.content, item, paragraphs, cx)),
        Footnote(note) | Endnote(note) => {
            Block::Group(build_blocks(&mut note.content, item, paragraphs, cx))
        }
        CodeBlock(code) => {
            let p = ir::Paragraph {
                content: vec![ir::InlineContent::Text(ir::TextSpan {
                    font_name: Some("monospace".into()),
                    ..ir::TextSpan::plain(&code.content)
                })],
                ..Default::default()
            };
            Block::Text(text(&p, None, item, paragraphs))
        }
        ThematicBreak => Block::Rule,
        _ => return None,
    };
    Some(block)
}

fn build_list(
    list: &mut ir::List,
    item: usize,
    paragraphs: &mut SearchParagraphs,
    cx: &mut App,
) -> Vec<Block> {
    let delimiter = list.ordered_delimiter();
    let mut blocks = Vec::new();
    for (index, entry) in list.items.iter_mut().enumerate() {
        let marker = if list.ordered {
            format!(
                "{}{delimiter} ",
                list.start_number.unwrap_or(1) as usize + index
            )
        } else {
            "• ".to_owned()
        };
        if let Some(ir::Element::Paragraph(first)) = entry.content.first_mut() {
            first
                .content
                .insert(0, ir::InlineContent::Text(ir::TextSpan::plain(marker)));
            let indent = first.indent_left_twips.get_or_insert(0);
            *indent = (*indent).max(i32::from(list.level) * 240);
        }
        blocks.extend(build_blocks(&mut entry.content, item, paragraphs, cx));
        if let Some(nested) = &mut entry.nested {
            blocks.extend(build_list(nested, item, paragraphs, cx));
        }
    }
    blocks
}

fn text(
    p: &ir::Paragraph,
    heading: Option<u8>,
    item: usize,
    paragraphs: &mut SearchParagraphs,
) -> Text {
    let mut html = String::from("<p>");
    let mut first = None;
    for content in &p.content {
        match content {
            ir::InlineContent::LineBreak => html.push_str("<br>"),
            ir::InlineContent::Text(span) => {
                if !span.text.trim().is_empty() {
                    first.get_or_insert(span);
                }
                let link = span
                    .hyperlink
                    .as_deref()
                    .and_then(|link| url::Url::parse(link).ok())
                    .filter(|link| matches!(link.scheme(), "http" | "https"));
                if let Some(link) = &link {
                    html.push_str("<a href=\"");
                    escape_html(&mut html, link.as_str());
                    html.push_str("\">");
                }
                if let Some([r, g, b]) = span.highlight {
                    let _ = write!(html, "<mark color=\"#{r:02x}{g:02x}{b:02x}\">");
                }
                let tags = [
                    ("strong", span.bold || heading.is_some()),
                    ("em", span.italic),
                    (
                        "u",
                        span.underline
                            .as_ref()
                            .is_some_and(|s| *s != ir::UnderlineStyle::None),
                    ),
                    ("s", span.strikethrough),
                ];
                for (tag, active) in tags {
                    if active {
                        let _ = write!(html, "<{tag}>");
                    }
                }
                if span.all_caps {
                    escape_html(&mut html, &span.text.to_uppercase());
                } else {
                    escape_html(&mut html, &span.text);
                }
                for (tag, active) in tags.into_iter().rev() {
                    if active {
                        let _ = write!(html, "</{tag}>");
                    }
                }
                if span.highlight.is_some() {
                    html.push_str("</mark>");
                }
                if link.is_some() {
                    html.push_str("</a>");
                }
            }
            ir::InlineContent::FootnoteRef(note) | ir::InlineContent::EndnoteRef(note) => {
                let _ = write!(html, "[{}]", note.note_id);
            }
            _ => {}
        }
    }
    html.push_str("</p>");
    let state = Rc::new(RefCell::new(ParagraphState::Pending(html)));
    paragraphs.push((item, state.clone()));
    let size = first
        .and_then(|span| span.font_size_half_pt)
        .map(|size| size as f32 * 2. / 3.)
        .unwrap_or(match heading {
            Some(1) => 27.,
            Some(2) => 22.,
            Some(_) => 18.,
            None => 15.,
        })
        .clamp(6., 160.);
    let marked_background = first.and_then(|span| span.highlight).filter(|&mark| {
        p.content.iter().all(|content| match content {
            ir::InlineContent::Text(span) => {
                span.text.trim().is_empty() || span.highlight == Some(mark)
            }
            _ => true,
        })
    });
    Text {
        state,
        size,
        font: first
            .and_then(|span| span.font_name.as_deref())
            .map(SharedString::from),
        color: first.and_then(|span| span.color),
        fill: p.background_color,
        marked_background,
        alignment: p.alignment.clone(),
        spacing: [
            twips(p.space_before_twips.unwrap_or(0)),
            twips(p.indent_right_twips.unwrap_or(0).max(0) as u32),
            twips(p.space_after_twips.unwrap_or(0)),
            twips(p.indent_left_twips.unwrap_or(0).max(0) as u32),
        ],
    }
}

fn escape_html(html: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            '\n' => html.push_str("<br>"),
            '\t' => html.push_str("&nbsp;&nbsp;&nbsp;&nbsp;"),
            character => html.push(character),
        }
    }
}

fn twips(value: u32) -> f32 {
    value.min(61_440) as f32 / 15.
}

fn build_table(
    table: &mut ir::Table,
    item: impl Fn(usize) -> usize,
    paragraphs: &mut SearchParagraphs,
    cx: &mut App,
) -> Table {
    let mut widths: Vec<_> = table
        .column_widths_twips
        .iter()
        .map(|&w| twips(w).max(1.))
        .collect();
    let mut busy = vec![0; widths.len()];
    let mut cells = Vec::new();
    let mut heights = Vec::new();
    let row_count = table.rows.len();
    let default_padding = table.cell_padding_twips.map_or(6., twips);
    for (row, source) in table.rows.iter_mut().enumerate() {
        heights.push(twips(source.height_twips.unwrap_or(0)));
        let mut column = 0;
        for source in &mut source.cells {
            let columns = source.col_span.max(1) as usize;
            let rows = (source.row_span.max(1) as usize).min(row_count - row);
            // The importer omits continuation cells. Reserve their grid slots
            // until the last row covered by the corresponding merged cell.
            loop {
                let end = column + columns;
                if end > busy.len() {
                    busy.resize(end, 0);
                    let default = source
                        .width_twips
                        .map_or(80., |w| twips(w) / columns as f32);
                    widths.resize(end, default.max(1.));
                }
                if let Some(offset) = busy[column..end].iter().position(|&until| until > row) {
                    column += offset + 1;
                } else {
                    busy[column..end].fill(row + rows);
                    break;
                }
            }
            let padding = source.padding.as_ref().map_or([default_padding; 4], |p| {
                [p.top_twips, p.right_twips, p.bottom_twips, p.left_twips]
                    .map(|n| n.map_or(default_padding, twips))
            });
            cells.push(Cell {
                row,
                column,
                columns,
                rows,
                padding,
                fill: source.background_color,
                vertical_align: source.vertical_align.clone(),
                blocks: build_blocks(&mut source.content, item(row), paragraphs, cx),
                continues: false,
            });
            column += columns;
        }
    }
    Table {
        widths: widths.into(),
        heights,
        cells,
        scroll: ScrollHandle::new(),
        first: true,
        last: true,
    }
}

fn color([r, g, b]: [u8; 3]) -> Hsla {
    rgb(u32::from_be_bytes([0, r, g, b])).into()
}

impl Block {
    fn suspend(&self, cx: &mut App) {
        match self {
            Self::Image(image, _) => image.update(cx, |image, cx| image.suspend(cx)),
            Self::Group(blocks) => blocks.iter().for_each(|block| block.suspend(cx)),
            Self::Table(table) => table
                .cells
                .iter()
                .flat_map(|cell| &cell.blocks)
                .for_each(|block| block.suspend(cx)),
            Self::Text(_) | Self::Rule => {}
        }
    }

    fn render(&self, scale: f32, paper: Hsla, cx: &mut App) -> AnyElement {
        match self {
            Block::Text(text) => text.render(scale, paper, cx).into_any_element(),
            Block::Table(table) => table.render(scale, paper, cx).into_any_element(),
            Block::Image(image, size) => h_flex()
                .w_full()
                .child(
                    img(PreparedImage::source(image))
                        .w(size.width * scale)
                        .max_w_full()
                        .aspect_ratio(f32::from(size.width) / f32::from(size.height))
                        .object_fit(ObjectFit::Contain),
                )
                .into_any_element(),
            Block::Group(blocks) => v_flex()
                .w_full()
                .children(blocks.iter().enumerate().map(|(index, block)| {
                    div()
                        .id(("word-group-block", index))
                        .child(block.render(scale, paper, cx))
                }))
                .into_any_element(),
            Block::Rule => div()
                .w_full()
                .h(px(1.))
                .bg(cx.theme().border)
                .into_any_element(),
        }
    }
}

impl Text {
    fn render(&self, scale: f32, paper: Hsla, cx: &mut App) -> Div {
        let fill = self.fill.map(color).unwrap_or(paper);
        let ink_background = self.marked_background.map(color).unwrap_or(fill);
        let ink = self
            .color
            .map(color)
            .filter(|ink| (ink_background.l - ink.l).abs() > 0.35)
            .unwrap_or_else(|| {
                let fallback = editor_ink(cx);
                if (ink_background.l - fallback.l).abs() > 0.45 {
                    fallback
                } else if ink_background.l > 0.55 {
                    black()
                } else {
                    white()
                }
            });
        let state = text_entity(&self.state, cx);
        let mut text = TextView::new(&state)
            .w_full()
            .selectable(true)
            .text_size(px(self.size * scale))
            .text_align(match self.alignment {
                Some(ir::ParagraphAlignment::Center) => TextAlign::Center,
                Some(ir::ParagraphAlignment::Right) => TextAlign::Right,
                _ => TextAlign::Left,
            })
            .style(
                TextViewStyle::from_theme(&gpui_kit::base::Theme::global(cx))
                    .with_foreground(ink)
                    .with_paragraph_gap(rems(0.)),
            )
            .on_link_click(|href, _, _, cx| {
                if href.starts_with("https://") || href.starts_with("http://") {
                    cx.open_url(href);
                }
            });
        if let Some(font) = &self.font {
            text = text.font_family(font.clone());
        }
        let [top, right, bottom, left] = self.spacing.map(|n| px(n * scale));
        div()
            .w_full()
            .min_w_0()
            .min_h(px(self.size * scale))
            .pt(top)
            .pr(right)
            .pb(bottom)
            .pl(left)
            .when(self.fill.is_some(), |element| element.bg(fill))
            .child(text)
    }
}

impl Table {
    fn into_bands(self) -> Vec<Self> {
        if self.heights.len() <= TABLE_ROWS_PER_BLOCK {
            return vec![self];
        }
        let count = self.heights.len().div_ceil(TABLE_ROWS_PER_BLOCK);
        let mut bands: Vec<_> = self
            .heights
            .chunks(TABLE_ROWS_PER_BLOCK)
            .enumerate()
            .map(|(index, heights)| Self {
                widths: self.widths.clone(),
                heights: heights.to_vec(),
                cells: Vec::new(),
                scroll: self.scroll.clone(),
                first: index == 0,
                last: index + 1 == count,
            })
            .collect();
        for mut cell in self.cells {
            let end = cell.row + cell.rows;
            let mut row = cell.row;
            // Continue the background and outer edge of a merged cell across
            // bands. Its searchable content lives in the first segment only.
            while row < end {
                let band = row / TABLE_ROWS_PER_BLOCK;
                let next = ((band + 1) * TABLE_ROWS_PER_BLOCK).min(end);
                bands[band].cells.push(Cell {
                    column: cell.column,
                    row: row % TABLE_ROWS_PER_BLOCK,
                    columns: cell.columns,
                    rows: next - row,
                    fill: cell.fill,
                    vertical_align: cell.vertical_align.clone(),
                    padding: cell.padding,
                    blocks: std::mem::take(&mut cell.blocks),
                    continues: next < end,
                });
                row = next;
            }
        }
        bands
    }

    fn render(&self, scale: f32, paper: Hsla, cx: &mut App) -> Stateful<Div> {
        let end = |index| i16::try_from(index).unwrap_or(i16::MAX);
        let grid = div()
            .grid()
            .flex_none()
            .w(px(self.widths.iter().sum::<f32>() * scale))
            .grid_cols_max_content(self.widths.len().try_into().unwrap_or(u16::MAX))
            .grid_rows_max_content((self.heights.len() + 1).try_into().unwrap_or(u16::MAX))
            // Zero-height measuring cells give each grid track its authored width.
            .children(self.widths.iter().enumerate().map(|(i, &width)| {
                div()
                    .col_start(end(i + 1))
                    .row_start(1)
                    .w(px(width * scale))
                    .h_0()
            }))
            .children(self.heights.iter().enumerate().map(|(i, &height)| {
                div()
                    .col_start(1)
                    .row_start(end(i + 2))
                    .h(px(height * scale))
                    .w_0()
            }))
            .children(self.cells.iter().map(|cell| {
                let width: f32 = self.widths[cell.column..cell.column + cell.columns]
                    .iter()
                    .sum();
                let [top, right, bottom, left] =
                    cell.padding.map(|n| px(n.min(width / 3.) * scale));
                let fill = cell.fill.map(color).unwrap_or(paper);
                v_flex()
                    .id(("word-cell", cell.row * self.widths.len() + cell.column))
                    .col_start(end(cell.column + 1))
                    .col_end(end(cell.column + cell.columns + 1))
                    .row_start(end(cell.row + 2))
                    .row_end(end(cell.row + cell.rows + 2))
                    .w(px(width * scale))
                    .min_w_0()
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .when(!cell.continues, |cell| cell.border_b_1())
                    .when(cell.row == 0 && self.first, |cell| cell.border_t_1())
                    .when(cell.column == 0, |cell| cell.border_l_1())
                    .bg(fill)
                    .pt(top)
                    .pr(right)
                    .pb(bottom)
                    .pl(left)
                    .when(
                        cell.vertical_align == Some(ir::CellVerticalAlign::Center),
                        |cell| cell.justify_center(),
                    )
                    .when(
                        cell.vertical_align == Some(ir::CellVerticalAlign::Bottom),
                        |cell| cell.justify_end(),
                    )
                    .children(cell.blocks.iter().enumerate().map(|(index, block)| {
                        div()
                            .id(("word-cell-block", index))
                            .child(block.render(scale, fill, cx))
                    }))
            }));
        div()
            .id("word-table")
            .relative()
            .w_full()
            .min_w_0()
            .child(
                div()
                    .id("word-table-scroll")
                    .w_full()
                    .min_w_0()
                    .overflow_x_scroll()
                    .track_scroll(&self.scroll)
                    .child(grid),
            )
            .child(Scrollbar::horizontal(&self.scroll).viewport_from_layout())
    }
}

#[cfg(test)]
mod tests {
    use super::{Block, ParagraphState, TABLE_ROWS_PER_BLOCK, WordDocument, WordView, ir};
    use gpui_kit::{
        AppContext, Bounds, Context, IntoElement, Render, TestAppContext, Window, WindowBounds,
        WindowOptions, point, px, size, test::TestWindowExt,
    };

    #[gpui_kit::test]
    fn long_table_bands_keep_merge_edges_and_search_item_positions(cx: &mut TestAppContext) {
        let rows_per_band = TABLE_ROWS_PER_BLOCK;
        let mut rows = vec![ir::TableRow::default(); rows_per_band * 2 + 1];
        let cell = |label, rows| ir::TableCell {
            row_span: rows,
            content: vec![ir::Element::Paragraph(ir::Paragraph {
                content: vec![ir::InlineContent::Text(ir::TextSpan::plain(label))],
                ..Default::default()
            })],
            ..Default::default()
        };
        rows[rows_per_band - 1].cells = vec![cell("Merged across bands", 2)];
        rows[rows_per_band].cells = vec![cell("Beside the merged cell", 1)];
        rows[rows_per_band * 2].cells = vec![cell("Final row", 1)];
        cx.update(|cx| {
            let view = WordView::new(
                WordDocument {
                    blocks: vec![ir::Element::Table(ir::Table {
                        column_widths_twips: vec![3000, 8400],
                        rows,
                        ..Default::default()
                    })],
                    memory: superfile_services::resources::pool()
                        .reserve_working(1, 1)
                        .unwrap(),
                    partial: true,
                },
                cx,
            );
            let [
                Block::Table(first),
                Block::Table(middle),
                Block::Table(last),
            ] = &*view.blocks
            else {
                panic!("expected three table bands");
            };
            assert_eq!(first.heights.len(), rows_per_band);
            assert_eq!(middle.heights.len(), rows_per_band);
            assert_eq!(last.heights.len(), 1);
            assert!(first.cells[0].continues);
            assert!(middle.cells[0].blocks.is_empty());
            assert_eq!(middle.cells[1].column, 1);
            assert!(view.partial);
            for ((item, state), expected) in view.paragraphs.iter().zip(0..) {
                assert_eq!(*item, expected);
                assert!(matches!(*state.borrow(), ParagraphState::Pending(_)));
            }
            assert_eq!(view.paragraphs.len(), 3);
        });
    }

    #[gpui_kit::test]
    fn merged_cells_keep_their_columns_and_wide_tables_scroll(cx: &mut TestAppContext) {
        struct Fixture(WordView);
        impl Render for Fixture {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                self.0.render(cx)
            }
        }

        let cell = |label, rows| ir::TableCell {
            row_span: rows,
            content: vec![ir::Element::Paragraph(ir::Paragraph {
                content: vec![ir::InlineContent::Text(ir::TextSpan::plain(label))],
                ..Default::default()
            })],
            ..Default::default()
        };
        let first = ir::TableRow {
            cells: vec![cell("Merged vertically", 2), cell("Top right", 1)],
            ..Default::default()
        };
        let second = ir::TableRow {
            cells: vec![cell("Bottom right", 1)],
            ..Default::default()
        };
        let source = ir::Table {
            column_widths_twips: vec![3000, 8400],
            rows: vec![first, second],
            ..Default::default()
        };
        let (window, fixture) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(420.), px(360.)),
                        cx,
                    ))),
                    ..Default::default()
                },
                cx,
                |_, cx| {
                    let memory = superfile_services::resources::pool()
                        .reserve_working(1, 1)
                        .unwrap();
                    let document = WordDocument {
                        blocks: vec![ir::Element::Table(source)],
                        memory,
                        partial: false,
                    };
                    cx.new(|cx| Fixture(WordView::new(document, cx)))
                },
            )
            .unwrap()
        });
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            let Block::Table(table) = &fixture.read(cx).0.blocks[0] else {
                panic!("expected table")
            };
            assert_eq!(table.cells[0].rows, 2);
            assert_eq!(table.cells[2].column, 1);
            assert_eq!(table.widths.as_ref(), [200., 560.]);
            let scroll = table.scroll.clone();
            let limit = scroll.max_offset().x;
            assert!(limit > px(0.));
            scroll.set_offset(point(-limit, px(0.)));
            window.render_frame(cx);
            assert_eq!(scroll.offset().x, -limit);
        })
        .unwrap();
    }
}
