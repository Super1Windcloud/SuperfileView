//! Unicode font discovery, subsetting, and PDF CID font metadata.

use std::collections::{BTreeMap, BTreeSet};

use fontdb::Database;
use subsetter::{subset, GlyphRemapper};
use ttf_parser::{Face, GlyphId};

use crate::{LoError, Result};

const FONT_OVERRIDE_ENV: &str = "LIBREOFFICE_PURE_UNICODE_FONT";

pub(crate) struct EmbeddedUnicodeFont {
    pub(crate) base_name: String,
    pub(crate) bytes: Vec<u8>,
    pub(crate) is_true_type: bool,
    pub(crate) widths: Vec<u16>,
    pub(crate) ascent: i16,
    pub(crate) descent: i16,
    pub(crate) bbox: (i16, i16, i16, i16),
    pub(crate) italic_angle: f32,
    pub(crate) char_to_code: BTreeMap<char, u16>,
    // Character codes identify Unicode scalars; CIDs identify subset glyphs.
    code_to_cid: Vec<u16>,
}

impl EmbeddedUnicodeFont {
    pub(crate) fn encode_hex(&self, text: &str) -> Result<String> {
        let mut out = String::with_capacity(text.chars().count() * 4 + 2);
        out.push('<');
        for ch in text.chars() {
            let code = self.char_to_code.get(&ch).ok_or_else(|| {
                LoError::Unsupported(format!(
                    "Unicode PDF font does not contain U+{:04X}",
                    ch as u32
                ))
            })?;
            out.push_str(&format!("{code:04X}"));
        }
        out.push('>');
        Ok(out)
    }

    pub(crate) fn widths_array(&self) -> String {
        self.widths
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub(crate) fn encoding_cmap(&self, name: &str) -> Option<Vec<u8>> {
        if self
            .code_to_cid
            .iter()
            .enumerate()
            .all(|(code, cid)| code == usize::from(*cid))
        {
            return None;
        }
        let mut map = format!("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> def\n/CMapName /{name} def\n/CMapType 1 def\n/WMode 0 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");
        for (chunk_index, chunk) in self.code_to_cid.chunks(100).enumerate() {
            map.push_str(&format!("{} begincidchar\n", chunk.len()));
            for (offset, cid) in chunk.iter().enumerate() {
                let code = chunk_index * 100 + offset;
                map.push_str(&format!("<{code:04X}> {cid}\n"));
            }
            map.push_str("endcidchar\n");
        }
        map.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        Some(map.into_bytes())
    }

    pub(crate) fn to_unicode_cmap(&self) -> Vec<u8> {
        let mut mappings = self
            .char_to_code
            .iter()
            .map(|(ch, cid)| (*cid, *ch))
            .collect::<Vec<_>>();
        mappings.sort_by_key(|(cid, _)| *cid);

        let mut cmap = String::from(
            "/CIDInit /ProcSet findresource begin\n\
             12 dict begin\n\
             begincmap\n\
             /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
             /CMapName /LOUnicode def\n\
             /CMapType 2 def\n\
             1 begincodespacerange\n\
             <0000> <FFFF>\n\
             endcodespacerange\n",
        );
        for chunk in mappings.chunks(100) {
            cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
            for (cid, ch) in chunk {
                cmap.push_str(&format!("<{cid:04X}> <{}>\n", utf16be_hex(*ch)));
            }
            cmap.push_str("endbfchar\n");
        }
        cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        cmap.into_bytes()
    }
}

// These formatting controls have no standalone painted outline. Preserve their
// text mapping without adding a visible box or advance between real glyphs.
fn invisible_control(ch: char) -> bool {
    matches!(ch as u32, 0x034f | 0x061c | 0x180b..=0x180f | 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f | 0xfe00..=0xfe0f | 0xfeff | 0xe0000..=0xe007f | 0xe0100..=0xe01ef)
}

pub(crate) fn build_unicode_fonts(chars: &BTreeSet<char>) -> Result<Vec<EmbeddedUnicodeFont>> {
    let mut database = Database::new();
    if let Some(paths) = std::env::var_os(FONT_OVERRIDE_ENV) {
        for path in std::env::split_paths(&paths) {
            database.load_font_file(&path).map_err(|error| {
                LoError::Io(format!(
                    "loading Unicode PDF font {}: {error}",
                    path.display()
                ))
            })?;
        }
    } else {
        database.load_system_fonts();
    }

    let mut candidates = database
        .faces()
        .filter(|face| {
            let is_last_resort = |name: &str| {
                name.chars()
                    .filter(char::is_ascii_alphanumeric)
                    .flat_map(char::to_lowercase)
                    .collect::<String>()
                    .starts_with("lastresort")
            };
            !is_last_resort(&face.post_script_name)
                && !face.families.iter().any(|(name, _)| is_last_resort(name))
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|face| std::cmp::Reverse(font_preference(face)));
    let mut remaining = chars.clone();
    let mut fonts = Vec::new();
    while !remaining.is_empty() {
        let mut selected = None;
        let mut selected_chars = BTreeSet::new();
        for &face in &candidates {
            let covered = database
                .with_face_data(face.id, |data, index| {
                    Face::parse(data, index)
                        .ok()
                        .filter(|parsed| {
                            // OpenType head.flags bit 14 marks cmap entries as generic
                            // last-resort symbols, even if the family was renamed.
                            let head = parsed
                                .raw_face()
                                .table(ttf_parser::Tag::from_bytes(b"head"));
                            let last_resort =
                                head.and_then(|h| h.get(16..18)).is_some_and(|flags| {
                                    u16::from_be_bytes([flags[0], flags[1]]) & (1 << 14) != 0
                                });
                            !last_resort
                                && (parsed.tables().glyf.is_some() || parsed.tables().cff.is_some())
                        })
                        .map(|parsed| {
                            let tables = parsed.tables();
                            let color_or_bitmap = tables.sbix.is_some()
                                || tables.cbdt.is_some()
                                || tables.svg.is_some()
                                || tables.colr.is_some()
                                || tables.ebdt.is_some()
                                || tables.bdat.is_some();
                            remaining
                                .iter()
                                .filter(|&&ch| {
                                    !invisible_control(ch)
                                        && parsed.glyph_index(ch).is_some_and(|glyph| {
                                            glyph.0 != 0
                                                && (!color_or_bitmap
                                                    || ch.is_whitespace()
                                                    || parsed.glyph_bounding_box(glyph).is_some())
                                        })
                                })
                                .take(u16::MAX as usize)
                                .copied()
                                .collect::<BTreeSet<_>>()
                        })
                        .unwrap_or_default()
                })
                .unwrap_or_default();
            if covered.len() > selected_chars.len() {
                selected = Some(face);
                selected_chars = covered;
                if selected_chars.len() == remaining.len().min(u16::MAX as usize) {
                    break;
                }
            }
        }
        let font = if let Some(selected) = selected {
            let result = database.with_face_data(selected.id, |data, index| {
                build_from_face(
                    data,
                    index,
                    &selected.post_script_name,
                    &selected_chars,
                    None,
                )
            });
            if let Some(Ok(font)) = result {
                font
            } else {
                // A face can advertise coverage but have unsupported or damaged
                // subset data. Let the next genuine font try those characters.
                candidates.retain(|face| face.id != selected.id);
                continue;
            }
        } else {
            // No installed outline exists for these scalars. Show a hollow box
            // and retain each original scalar in ToUnicode for search/copy.
            selected_chars = remaining
                .iter()
                .take(usize::from(u16::MAX))
                .copied()
                .collect();
            build_from_face(
                include_bytes!("../fonts/missing-glyph.ttf"),
                0,
                "SuperfileMissingGlyph",
                &selected_chars,
                Some('\u{fffd}'),
            )?
        };
        remaining.retain(|ch| !selected_chars.contains(ch));
        fonts.push(font);
    }
    Ok(fonts)
}

fn build_from_face(
    data: &[u8],
    face_index: u32,
    post_script_name: &str,
    chars: &BTreeSet<char>,
    replacement: Option<char>,
) -> Result<EmbeddedUnicodeFont> {
    let face = Face::parse(data, face_index)
        .map_err(|_| LoError::Parse("selected Unicode PDF font is malformed".to_string()))?;
    let mut remapper = GlyphRemapper::new();
    let mut char_to_code = BTreeMap::new();
    let mut code_to_cid = vec![0];
    for ch in chars {
        let drawn = match replacement {
            Some(_) if invisible_control(*ch) => '\u{200b}',
            Some(fallback) => fallback,
            None => *ch,
        };
        let glyph = face.glyph_index(drawn).ok_or_else(|| {
            LoError::Unsupported(format!(
                "selected Unicode PDF font is missing U+{:04X}",
                *ch as u32
            ))
        })?;
        let code = u16::try_from(code_to_cid.len())
            .map_err(|_| LoError::Unsupported("Unicode font exceeds PDF codespace".into()))?;
        char_to_code.insert(*ch, code);
        code_to_cid.push(remapper.remap(glyph.0));
    }

    let units_per_em = face.units_per_em().max(1) as u32;
    let widths = remapper
        .remapped_gids()
        .map(|old_gid| {
            let advance = face
                .glyph_hor_advance(GlyphId(old_gid))
                .unwrap_or(units_per_em as u16);
            ((advance as u32 * 1000 + units_per_em / 2) / units_per_em).min(u16::MAX as u32) as u16
        })
        .collect::<Vec<_>>();
    let bbox = face.global_bounding_box();
    let scale_metric = |value: i16| -> i16 {
        ((value as i32 * 1000) / units_per_em as i32).clamp(i16::MIN as i32, i16::MAX as i32) as i16
    };
    let subset = subset(data, face_index, &remapper)
        .map_err(|error| LoError::Unsupported(format!("subsetting Unicode PDF font: {error}")))?;
    let is_true_type = has_sfnt_table(&subset, b"glyf");
    if !is_true_type && !has_sfnt_table(&subset, b"CFF ") {
        return Err(LoError::Unsupported(
            "Unicode PDF font subset has neither TrueType nor CFF outlines".to_string(),
        ));
    }

    Ok(EmbeddedUnicodeFont {
        base_name: format!("LOUNIC+{}", pdf_name(post_script_name)),
        bytes: subset,
        is_true_type,
        widths,
        ascent: scale_metric(face.ascender()),
        descent: scale_metric(face.descender()),
        bbox: (
            scale_metric(bbox.x_min),
            scale_metric(bbox.y_min),
            scale_metric(bbox.x_max),
            scale_metric(bbox.y_max),
        ),
        italic_angle: face.italic_angle(),
        char_to_code,
        code_to_cid,
    })
}

fn font_preference(face: &fontdb::FaceInfo) -> u8 {
    let names = face
        .families
        .iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .chain(std::iter::once(face.post_script_name.to_ascii_lowercase()))
        .collect::<Vec<_>>()
        .join(" ");
    for (needle, score) in [
        ("noto sans cjk sc", 100),
        ("noto sans sc", 95),
        ("source han sans sc", 90),
        ("arial unicode", 85),
        ("pingfang sc", 80),
        ("hiragino sans gb", 75),
        ("microsoft yahei", 72),
        ("droid sans fallback", 70),
        ("wenquanyi", 65),
    ] {
        if names.contains(needle) {
            return score;
        }
    }
    0
}

fn pdf_name(value: &str) -> String {
    let cleaned = value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
        .collect::<String>();
    if cleaned.is_empty() {
        "UnicodeFont".to_string()
    } else {
        cleaned
    }
}

fn has_sfnt_table(bytes: &[u8], wanted: &[u8; 4]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    (0..count).any(|index| {
        let start = 12 + index * 16;
        bytes.get(start..start + 4) == Some(wanted.as_slice())
    })
}

fn utf16be_hex(ch: char) -> String {
    let mut units = [0_u16; 2];
    ch.encode_utf16(&mut units)
        .iter()
        .map(|unit| format!("{unit:04X}"))
        .collect::<String>()
}
