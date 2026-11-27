//! Table presentation consumes admitted cells and emits semantic page requests.
use crate::views::reader::contract::ReaderHost;
#[cfg(test)]
use gpui_kit::TestSupportExt as _;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    base::InteractiveElementExt as _,
    component::{ActiveTheme, Disableable, button::Button, h_flex, scroll::Scrollbar, v_flex},
    *,
};
use std::collections::HashMap;
use superfile_core::table::TablePage;

#[derive(Clone, Copy)]
pub(super) enum TableRequest {
    Retry,
    PreviousRows,
    NextRows,
    PreviousColumns,
    NextColumns,
}

pub(super) struct TableView {
    host: ReaderHost,
    show_detail: fn(String, String, &mut Window, &mut App),
    columns_limit: usize,
    cell_chars: usize,
    pub(super) page: TablePage,
    image_urls: HashMap<String, url::Url>,
    images: HashMap<String, Entity<super::super::media::InlineImageReader>>,
    format: &'static str,
    row_start: usize,
    column_start: usize,
    has_previous: bool,
    busy: bool,
    error: Option<String>,
    resume_scroll: Option<(Point<Pixels>, Point<Pixels>)>,
    scroll: UniformListScrollHandle,
    horizontal_scroll: ScrollHandle,
    text_scale: f32,
}
impl EventEmitter<TableRequest> for TableView {}
impl TableView {
    pub fn new(
        host: ReaderHost,
        columns_limit: usize,
        cell_chars: usize,
        show_detail: fn(String, String, &mut Window, &mut App),
    ) -> Self {
        Self {
            host,
            show_detail,
            columns_limit,
            cell_chars,
            page: TablePage::default(),
            image_urls: HashMap::new(),
            images: HashMap::new(),
            format: "CSV",
            row_start: 0,
            column_start: 0,
            has_previous: false,
            busy: false,
            error: None,
            resume_scroll: None,
            scroll: UniformListScrollHandle::new(),
            horizontal_scroll: ScrollHandle::new(),
            text_scale: 1.,
        }
    }
    pub fn reset_position(&mut self) {
        self.resume_scroll = None;
    }
    fn set_text_scale(&mut self, scale: f32) {
        if self.text_scale == scale {
            return;
        }
        let ratio = scale / self.text_scale;
        self.text_scale = scale;
        let handle = self.scroll.0.borrow().base_handle.clone();
        handle.set_offset(point(px(0.), handle.offset().y * ratio));
        self.horizontal_scroll
            .set_offset(point(self.horizontal_scroll.offset().x * ratio, px(0.)));
    }
    pub fn begin(&mut self, format: &'static str, row: usize, column: usize, has_previous: bool) {
        self.format = format;
        self.row_start = row;
        self.column_start = column;
        self.has_previous = has_previous;
        self.page = TablePage::default();
        self.image_urls.clear();
        self.images.clear();
        self.error = None;
        self.busy = true;
    }
    pub fn publish(&mut self, page: TablePage, image_urls: HashMap<String, url::Url>) {
        self.page = page;
        self.image_urls = image_urls;
        self.images.clear();
        self.busy = false;
        self.scroll = UniformListScrollHandle::new();
        if let Some((vertical, horizontal)) = self.resume_scroll.take() {
            self.scroll.0.borrow().base_handle.set_offset(vertical);
            self.horizontal_scroll.set_offset(horizontal);
        } else {
            self.horizontal_scroll.set_offset(point(px(0.), px(0.)));
        }
    }
    pub fn clear_images(&mut self, cx: &mut App) {
        for (_, image) in self.images.drain() {
            image.update(cx, |image, cx| image.set_active(false, cx));
        }
    }
    fn ensure_image(
        &mut self,
        url: &url::Url,
        cx: &mut Context<Self>,
    ) -> Entity<super::super::media::InlineImageReader> {
        if let Some(existing) = self.images.get(url.as_str()) {
            return existing.clone();
        }
        let target = url.clone();
        let entity = cx.new(|cx| {
            let mut reader = super::super::media::InlineImageReader::new(target, cx);
            reader.set_active(true, cx);
            reader
        });
        self.images.insert(url.as_str().to_owned(), entity.clone());
        entity
    }
    pub fn fail(&mut self, error: String) {
        self.busy = false;
        self.error = Some(error);
    }
    pub fn suspend(&mut self, remember_scroll: bool) {
        if remember_scroll && !self.busy && self.error.is_none() {
            self.resume_scroll = Some((
                self.scroll.0.borrow().base_handle.offset(),
                self.horizontal_scroll.offset(),
            ));
        }
        self.page = TablePage::default();
        self.images.clear();
        self.busy = false;
    }
    fn request(&self, request: TableRequest, cx: &mut Context<Self>) {
        let action = match request {
            TableRequest::Retry => "retry",
            TableRequest::PreviousRows => "previous_rows",
            TableRequest::NextRows => "next_rows",
            TableRequest::PreviousColumns => "previous_columns",
            TableRequest::NextColumns => "next_columns",
        };
        (self.host.action)("csv", action, &[]);
        cx.emit(request);
    }
    fn show_cell(&self, row: usize, column: usize, window: &mut Window, cx: &mut Context<Self>) {
        (self.host.action)("csv", "cell_detail", &[]);
        let Some(value) = self
            .page
            .rows
            .get(row)
            .and_then(|row| row.get(column))
            .cloned()
        else {
            return;
        };
        (self.show_detail)((self.host.translate)("Full cell value"), value, window, cx);
    }
}
impl Render for TableView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy;
        if busy {
            return super::super::controls::loading((self.host.translate)("Loading preview…"), cx);
        }
        let columns = self
            .page
            .columns
            .saturating_sub(self.column_start)
            .min(self.columns_limit);
        let base_size = cx.theme().font_size;
        let font_size = (self.host.font_size)(base_size, cx);
        let scale = f32::from(font_size) / f32::from(base_size);
        self.set_text_scale(scale);
        let row_height = px(32.) * scale;
        let row_width = px(64.) * scale;
        let cell_width = px(200.) * scale;
        let width = row_width + cell_width * columns as f32;
        let geographic = matches!(self.format, "GPX" | "KML");
        let horizontal = self.horizontal_scroll.clone();
        let vertical = self.scroll.0.borrow().base_handle.clone();
        let view = cx.entity().downgrade();
        v_flex().relative().size_full().min_h_0()
            .child(canvas(
                |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |_, hitbox, window, _| {
                    let horizontal = horizontal.clone();
                    let vertical = vertical.clone();
                    let view = view.clone();
                    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture
                            || !hitbox.should_handle_scroll(window)
                            || !horizontal.bounds().contains(&event.position) {
                            return;
                        }
                        let delta = event.delta.pixel_delta(window.line_height());
                        if event.modifiers.shift {
                            // macOS may already map Shift-wheel's vertical delta to x.
                            let distance = if delta.x.abs() > delta.y.abs() { delta.x } else { delta.y };
                            let offset = horizontal.offset();
                            horizontal.set_offset(point(
                                (offset.x + distance).clamp(-horizontal.max_offset().x.max(px(0.)), px(0.)),
                                px(0.),
                            ));
                        } else {
                            let offset = vertical.offset();
                            vertical.set_offset(point(px(0.),
                                (offset.y + delta.y).clamp(-vertical.max_offset().y.max(px(0.)), px(0.))));
                        }
                        // Consume even at an edge: the nested scrollers must not
                        // reinterpret a remaining delta as movement on the other axis.
                        cx.stop_propagation();
                        let _ = view.update(cx, |_, cx| cx.notify());
                    });
                },
            ).absolute().size_full())

            .child(div().id("csv-horizontal").flex_1().min_h_0().overflow_x_scroll().track_scroll(&self.horizontal_scroll).child(
                v_flex().w(width).h_full().text_size(font_size)
                    .child(h_flex().h(row_height).child(div().w(row_width).flex_shrink_0().px_2().child((self.host.translate)("Row")))
                        .children((0..columns).map(|column| div().w(cell_width).px_3().child(if geographic {
                            (self.host.translate)(match column { 0 => "Longitude", 1 => "Latitude", _ => "Altitude" })
                        } else { (self.column_start + column + 1).to_string() }))))
                    .child(uniform_list("csv-rows", self.page.rows.len() + 3, cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                        let thumb_size = (row_height - px(6.)).max(px(16.));
                        range.map(|row| {
                            if row >= view.page.rows.len() { return div().h(row_height).into_any_element(); }
                            h_flex().h(row_height).w_full().overflow_hidden().border_b_1().border_color(cx.theme().border)
                                .child(div().w(row_width).flex_shrink_0().px_2().text_color(cx.theme().muted_foreground)
                                    .child((view.row_start + row + 1).to_string()))
                                .children((0..columns).map(|column| {
                                    let value = view.page.rows[row].get(column).map(String::as_str).unwrap_or("");
                                    let preview = value.chars().take(view.cell_chars)
                                        .map(|ch| if matches!(ch, '\n' | '\r' | '\t' | '\u{0085}' | '\u{2028}' | '\u{2029}') { ' ' } else { ch })
                                        .collect::<String>();
                                    let image_url = view.image_urls.get(value).cloned();
                                    let body = if let Some(url) = image_url {
                                        let image = view.ensure_image(&url, cx);
                                        let source = super::super::media::inline_image_source(image);
                                        let detail = source.clone();
                                        h_flex()
                                            .gap_2()
                                            .w_full()
                                            .min_w_0()
                                            .items_center()
                                            .child(
                                                div()
                                                    .id(("csv-cell-image", row * columns + column))
                                                    .w(thumb_size)
                                                    .h(thumb_size)
                                                    .flex_shrink_0()
                                                    .rounded(px(4.))
                                                    .overflow_hidden()
                                                    .bg(cx.theme().muted.opacity(0.35))
                                                    .cursor_pointer()
                                                    .on_click(move |_, window, cx| {
                                                        cx.stop_propagation();
                                                        super::super::image_detail::show(detail.clone(), window, cx);
                                                    })
                                                    .child(img(source).size_full().object_fit(ObjectFit::Contain)),
                                            )
                                            .child(div().flex_1().min_w_0().truncate().child(preview))
                                            .into_any_element()
                                    } else {
                                        div().w_full().min_w_0().truncate().child(preview).into_any_element()
                                    };
                                    let cell = div().id(("csv-cell", row * columns + column)).w(cell_width).h_full().flex_shrink_0().px_3()
                                        .flex().items_center().overflow_hidden()
                                        .child(body)
                                        .on_double_click(cx.listener(move |view, _, window, cx| view.show_cell(row, column, window, cx)));
                                    #[cfg(test)]
                                    let cell = cell.test_support();
                                    cell.into_any_element()
                                })).into_any_element()
                        }).collect()
                    })).track_scroll(&self.scroll).flex_1().min_h_0())
            ))
            .child(div().h(Scrollbar::width()).flex_shrink_0().child(
                Scrollbar::horizontal(&self.horizontal_scroll).viewport_from_layout(),
            )).child(super::super::controls::floating("csv-controls", h_flex().p_0().gap_2().text_xs()
                .child(div().min_w_0().truncate().child(if busy { (self.host.translate)("Loading table…") } else {
                    self.error.clone().unwrap_or_else(|| format!("{} · rows {}–{} · columns {}–{}", self.format,
                        self.row_start + 1, self.row_start + self.page.rows.len(), self.column_start + 1, self.column_start + columns))
                }))
                .when(self.error.is_some(), |bar| bar.child(
                    Button::new("csv-retry").label((self.host.translate)("Retry"))
                        .on_click(cx.listener(|view, _, _, cx| view.request(TableRequest::Retry, cx)))
                ))
                .when(self.has_previous || self.page.more, |bar| bar
                .child(crate::views::icon_button("csv-prev", gpui_kit::assets::IconName::ChevronLeft, (self.host.translate)("Previous"))
                    .disabled(busy || !self.has_previous).on_click(cx.listener(|view, _, _, cx| view.request(TableRequest::PreviousRows, cx))))
                .child(crate::views::icon_button("csv-next", gpui_kit::assets::IconName::ChevronRight, (self.host.translate)("Next page"))
                    .disabled(busy || !self.page.more).on_click(cx.listener(|view, _, _, cx| view.request(TableRequest::NextRows, cx))))
                )
                .when(self.column_start > 0 || self.column_start + columns < self.page.columns, |bar| bar
                .child(Button::new("csv-prev-cols").label((self.host.translate)("Previous columns"))
                    .disabled(busy || self.column_start == 0).on_click(cx.listener(|view, _, _, cx| view.request(TableRequest::PreviousColumns, cx))))
                .child(Button::new("csv-next-cols").label((self.host.translate)("Next columns"))
                    .disabled(busy || self.column_start + columns >= self.page.columns).on_click(cx.listener(|view, _, _, cx| view.request(TableRequest::NextColumns, cx))))), cx)).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{ReaderHost, TablePage, TableView};
    use gpui_kit::{point, px};
    use superfile_core::resources::{ResourcePool, TextRowsBuilder};

    fn contract_page(pool: &ResourcePool) -> TablePage {
        let expected: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../../../assets/reader-contracts/table.json"
        ))
        .unwrap();
        let mut rows = TextRowsBuilder::new(pool);
        for row in expected["rows"].as_array().unwrap() {
            rows.try_push(
                row.as_array()
                    .unwrap()
                    .iter()
                    .map(|cell| cell.as_str().unwrap()),
            )
            .unwrap();
        }
        TablePage {
            rows: rows.finish(),
            columns: expected["columns"].as_u64().unwrap() as usize,
            more: expected["more"].as_bool().unwrap(),
        }
    }

    #[test]
    fn contract_handoff_and_repeated_suspension_preserve_values_and_scroll() {
        let pool = ResourcePool::with_budgets(0, 16 * 1024);
        let host = ReaderHost {
            translate: str::to_owned,
            action: |_, _, _| {},
            font_size: |size, _| size,
            resources: pool.clone(),
        };
        let mut view = TableView::new(host, 3, 10, |_, _, _, _| {});
        let page = contract_page(&pool);
        let original = page.rows[1][2].as_ptr();
        let bytes = pool.snapshot().working_bytes;
        view.begin("CSV", 120, 4, true);
        view.publish(page, Default::default());
        assert_eq!(view.page.rows[1][2].as_ptr(), original);
        assert_eq!(view.page.rows[1][2], "line 1\nline 2 \"yes\"");
        assert_eq!(view.page.rows[2], ["short"]);
        assert_eq!(pool.snapshot().working_bytes, bytes);
        let vertical = point(px(0.), px(-64.));
        let horizontal = point(px(-120.), px(0.));
        view.scroll.0.borrow().base_handle.set_offset(vertical);
        view.horizontal_scroll.set_offset(horizontal);
        view.suspend(true);
        view.suspend(false);
        assert_eq!(pool.snapshot().working_bytes, 0);
        assert_eq!(
            (view.row_start, view.column_start, view.has_previous),
            (120, 4, true)
        );
        view.begin("CSV", 120, 4, true);
        view.fail("retry".into());
        assert!(view.page.rows.is_empty());
        view.begin("CSV", 120, 4, true);
        view.publish(contract_page(&pool), Default::default());
        assert_eq!(view.scroll.0.borrow().base_handle.offset(), vertical);
        assert_eq!(view.horizontal_scroll.offset(), horizontal);
        drop(view);
        assert_eq!(pool.snapshot().working_bytes, 0);
    }

    #[gpui_kit::test]
    fn table_view_renders_cell_image_thumbnails(cx: &mut gpui_kit::TestAppContext) {
        use gpui_kit::{AppContext, WindowOptions, test::TestWindowExt};
        use std::collections::HashMap;
        use superfile_core::resources::{ResourcePool, TextRowsBuilder};

        cx.update(gpui_kit::init);
        let data_uri = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
        let image_url = url::Url::parse(data_uri).unwrap();
        let pool = ResourcePool::with_budgets(0, 16 * 1024);
        let mut rows = TextRowsBuilder::new(&pool);
        rows.try_push(["item", data_uri].into_iter()).unwrap();
        let page = TablePage {
            rows: rows.finish(),
            columns: 2,
            more: false,
        };
        let mut image_urls = HashMap::new();
        image_urls.insert(data_uri.to_owned(), image_url);

        let (handle, view) = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |_, cx| {
                cx.new(|_| {
                    let host = ReaderHost {
                        translate: str::to_owned,
                        action: |_, _, _| {},
                        font_size: |size, _| size,
                        resources: pool.clone(),
                    };
                    let mut view = TableView::new(host, 10, 64, |_, _, _, _| {});
                    view.begin("CSV", 0, 0, false);
                    view.publish(page, image_urls);
                    view
                })
            })
            .unwrap()
        });
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            assert_eq!(view.read(cx).images.len(), 1);
            view.update(cx, |view, cx| view.clear_images(cx));
            assert!(view.read(cx).images.is_empty());
        });
    }
}
