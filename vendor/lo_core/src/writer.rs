use crate::geometry::Size;
use crate::meta::Metadata;
use crate::style::{PageStyle, ParagraphStyle, TextStyle};
use crate::units::Length;

#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Bold(String),
    Italic(String),
    Code(String),
    Link {
        label: String,
        url: String,
    },
    Styled {
        text: String,
        style: TextStyle,
        url: Option<String>,
    },
    LineBreak,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paragraph {
    pub style: ParagraphStyle,
    pub text_style: TextStyle,
    pub spans: Vec<Inline>,
}

impl Paragraph {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            spans: vec![Inline::Text(text.into())],
            ..Self::default()
        }
    }

    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        for span in &self.spans {
            match span {
                Inline::Text(text)
                | Inline::Bold(text)
                | Inline::Italic(text)
                | Inline::Code(text)
                | Inline::Styled { text, .. } => out.push_str(text),
                Inline::Link { label, .. } => out.push_str(label),
                Inline::LineBreak => out.push('\n'),
            }
        }
        out
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Heading {
    pub level: u8,
    pub content: Paragraph,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListItem {
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ListBlock {
    pub ordered: bool,
    pub items: Vec<ListItem>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableCell {
    pub paragraphs: Vec<Paragraph>,
    /// Inline drawings are retained at their boundary between text blocks.
    pub images: Vec<CellImage>,
    pub tables: Vec<CellTable>,
    /// Zero denotes the default single grid column.
    pub grid_span: u16,
    pub fill: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellImage {
    pub before_paragraph: usize,
    pub alignment: crate::Alignment,
    pub image: ImageBlock,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CellTable {
    pub before_paragraph: usize,
    pub table: Table,
}

pub enum CellContent<'a> {
    Paragraph(&'a Paragraph),
    Image(&'a CellImage),
    Table(&'a Table),
}

impl TableCell {
    pub fn contents(&self) -> impl Iterator<Item = CellContent<'_>> {
        let mut images = self.images.iter().peekable();
        let mut tables = self.tables.iter().peekable();
        let mut paragraph = 0;
        std::iter::from_fn(move || {
            if images.peek().is_some_and(|image| {
                (image.before_paragraph <= paragraph || paragraph == self.paragraphs.len())
                    && tables.peek().is_none_or(|table| {
                        image.before_paragraph <= table.before_paragraph
                    })
            }) {
                return images.next().map(CellContent::Image);
            }
            if tables.peek().is_some_and(|table| {
                table.before_paragraph <= paragraph || paragraph == self.paragraphs.len()
            }) {
                return tables.next().map(|table| CellContent::Table(&table.table));
            }
            let next = self.paragraphs.get(paragraph)?;
            paragraph += 1;
            Some(CellContent::Paragraph(next))
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
    pub grid_before: u16,
    pub repeat_header: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    /// DOCX layout grid widths in twentieths of a point. Other importers
    /// leave this unset to retain their content-based table styling.
    pub docx_grid: Option<Vec<u32>>,
    pub name: String,
    pub rows: Vec<TableRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ImageBlock {
    pub name: String,
    pub mime_type: String,
    pub data: Vec<u8>,
    pub alt: String,
    pub size: Size,
}

impl Default for ImageBlock {
    fn default() -> Self {
        Self {
            name: "image.bin".to_string(),
            mime_type: "application/octet-stream".to_string(),
            data: Vec::new(),
            alt: String::new(),
            size: Size::new(Length::mm(50.0), Length::mm(30.0)),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Section {
    pub name: String,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(Heading),
    Paragraph(Paragraph),
    List(ListBlock),
    Table(Table),
    Image(ImageBlock),
    Section(Section),
    PageBreak,
    HorizontalRule,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextDocument {
    pub meta: Metadata,
    pub page_style: PageStyle,
    pub body: Vec<Block>,
}

impl Default for TextDocument {
    fn default() -> Self {
        Self {
            meta: Metadata::default(),
            page_style: PageStyle {
                width_mm: 210,
                height_mm: 297,
                margin_mm: 20,
                section: None,
            },
            body: Vec::new(),
        }
    }
}

impl TextDocument {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            meta: Metadata::titled(title),
            ..Self::default()
        }
    }

    pub fn push_paragraph(&mut self, text: impl Into<String>) -> &mut Self {
        self.body.push(Block::Paragraph(Paragraph::plain(text)));
        self
    }

    pub fn push_heading(&mut self, level: u8, text: impl Into<String>) -> &mut Self {
        self.body.push(Block::Heading(Heading {
            level,
            content: Paragraph::plain(text),
        }));
        self
    }

    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for block in &self.body {
            match block {
                Block::Paragraph(p) => {
                    out.push_str(&p.to_plain_text());
                    out.push_str("\n\n");
                }
                Block::Heading(h) => {
                    out.push_str(&h.content.to_plain_text());
                    out.push_str("\n\n");
                }
                Block::List(list) => {
                    for item in &list.items {
                        out.push_str("- ");
                        for nested in &item.blocks {
                            if let Block::Paragraph(p) = nested {
                                out.push_str(&p.to_plain_text());
                            }
                        }
                        out.push('\n');
                    }
                    out.push('\n');
                }
                Block::Table(table) => {
                    out.push_str(&table_text(table));
                    out.push('\n');
                }
                Block::Image(image) => {
                    out.push_str(&format!("[image: {}]\n\n", image.alt));
                }
                Block::Section(section) => {
                    out.push_str(&format!("[section: {}]\n", section.name));
                    for nested in &section.blocks {
                        if let Block::Paragraph(p) = nested {
                            out.push_str(&p.to_plain_text());
                            out.push('\n');
                        }
                    }
                    out.push('\n');
                }
                Block::PageBreak => out.push_str("\n--- page break ---\n\n"),
                Block::HorizontalRule => out.push_str("\n---\n\n"),
            }
        }
        out.trim_end().to_string()
    }

    pub fn embedded_images(&self) -> Vec<(String, String, Vec<u8>)> {
        fn collect(blocks: &[Block], images: &mut Vec<(String, String, Vec<u8>)>) {
            for block in blocks {
                match block {
                    Block::Image(image) => images.push((
                        image.name.clone(),
                        image.mime_type.clone(),
                        image.data.clone(),
                    )),
                    Block::Table(table) => {
                        collect_table_images(table, images);
                    }
                    Block::Section(section) => collect(&section.blocks, images),
                    _ => {}
                }
            }
        }

        let mut images = Vec::new();
        collect(&self.body, &mut images);
        images
    }
}

pub fn table_text(table: &Table) -> String {
    let mut output = String::new();
    for row in &table.rows {
        let cells = row.cells.iter().map(|cell| {
            cell.contents().map(|content| match content {
                CellContent::Paragraph(paragraph) => paragraph.to_plain_text(),
                CellContent::Image(figure) => format!("[image: {}]", figure.image.alt),
                CellContent::Table(table) => table_text(table).trim_end().to_owned(),
            }).collect::<Vec<_>>().join(" ")
        }).collect::<Vec<_>>();
        output.push_str(&cells.join(" | "));
        output.push('\n');
    }
    output
}

fn collect_table_images(table: &Table, images: &mut Vec<(String, String, Vec<u8>)>) {
    for cell in table.rows.iter().flat_map(|row| &row.cells) {
        for content in cell.contents() {
            match content {
                CellContent::Image(figure) if !figure.image.data.is_empty() => {
                    let image = &figure.image;
                    images.push((image.name.clone(), image.mime_type.clone(), image.data.clone()));
                }
                CellContent::Table(table) => collect_table_images(table, images),
                _ => {}
            }
        }
    }
}
