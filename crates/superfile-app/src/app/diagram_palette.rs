use superfile_core::reader_palette::ReaderPalette;

pub(super) fn mermaid(source: &str, palette: Option<ReaderPalette>) -> anyhow::Result<String> {
    use mermaid_rs_renderer::{RenderOptions, Theme, render_with_options};
    let Some(palette) = palette else {
        return mermaid_rs_renderer::render(source);
    };
    let paper = ReaderPalette::hex(palette.paper);
    let ink = ReaderPalette::hex(palette.ink);
    let surface = ReaderPalette::hex(palette.surface);
    let accent = ReaderPalette::hex(palette.accent);
    let mut theme = if palette.is_dark() {
        Theme::dark()
    } else {
        Theme::modern()
    };
    for field in [
        &mut theme.background,
        &mut theme.edge_label_background,
        &mut theme.git_commit_label_background,
        &mut theme.git_tag_label_background,
    ] {
        field.clone_from(&paper);
    }
    for field in [
        &mut theme.primary_color,
        &mut theme.secondary_color,
        &mut theme.tertiary_color,
        &mut theme.cluster_background,
        &mut theme.sequence_actor_fill,
        &mut theme.sequence_note_fill,
        &mut theme.sequence_activation_fill,
    ] {
        field.clone_from(&surface);
    }
    for field in [
        &mut theme.text_color,
        &mut theme.primary_text_color,
        &mut theme.line_color,
        &mut theme.sequence_actor_line,
        &mut theme.pie_title_text_color,
        &mut theme.pie_legend_text_color,
        &mut theme.git_tag_label_color,
        &mut theme.git_commit_label_color,
    ] {
        field.clone_from(&ink);
    }
    for field in [
        &mut theme.primary_border_color,
        &mut theme.cluster_border,
        &mut theme.sequence_note_border,
        &mut theme.sequence_actor_border,
        &mut theme.sequence_activation_border,
        &mut theme.git_tag_label_border,
    ] {
        field.clone_from(&accent);
    }
    render_with_options(
        source,
        RenderOptions {
            theme,
            ..Default::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flowchart_uses_editor_text_surface_and_accent() {
        let palette = ReaderPalette {
            paper: [0x10, 0x20, 0x30],
            ink: [0xc8, 0xd8, 0xe8],
            surface: [0x20, 0x30, 0x40],
            accent: [0xa0, 0x60, 0xb0],
        };
        let svg = mermaid("flowchart LR\nA[Alpha] --> B[Beta]", Some(palette)).unwrap();
        for color in [palette.ink, palette.surface, palette.accent] {
            assert!(svg.contains(&ReaderPalette::hex(color)), "{svg}");
        }
    }
}
