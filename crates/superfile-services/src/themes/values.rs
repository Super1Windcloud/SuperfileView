use super::CssColor;
use anyhow::{Result, anyhow, ensure};
use color::{ColorSpaceTag, DynamicColor, HueDirection};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(super) fn split_top(text: &str, delimiter: char) -> Vec<&str> {
    let mut depth = 0;
    let mut start = 0;
    let mut result = Vec::new();
    let mut quote = None;
    let mut escape = false;
    for (index, ch) in text.char_indices() {
        if escape {
            escape = false;
            continue;
        }
        if ch == '\\' {
            escape = true;
            continue;
        }
        if let Some(current) = quote {
            if ch == current {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ if ch == delimiter && depth == 0 => {
                result.push(&text[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    result.push(&text[start..]);
    result
}

pub(super) fn closing(text: &str, open: usize, left: u8, right: u8) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut quote = None;
    let mut escape = false;
    for (index, byte) in bytes.iter().copied().enumerate().skip(open) {
        if escape {
            escape = false;
            continue;
        }
        if byte == b'\\' {
            escape = true;
            continue;
        }
        if let Some(current) = quote {
            if byte == current {
                quote = None;
            }
            continue;
        }
        if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
            continue;
        }
        if byte == left {
            depth += 1;
        }
        if byte == right {
            depth = depth.checked_sub(1)?;
            if depth == 0 {
                return Some(index);
            }
        }
    }
    None
}

pub(super) struct Resolver<'a> {
    pub vars: &'a BTreeMap<String, String>,
    pub overrides: &'a BTreeSet<String>,
    cache: HashMap<String, Option<(String, bool)>>,
    stack: BTreeSet<String>,
}
impl<'a> Resolver<'a> {
    pub fn new(vars: &'a BTreeMap<String, String>, overrides: &'a BTreeSet<String>) -> Self {
        Self {
            vars,
            overrides,
            cache: HashMap::new(),
            stack: BTreeSet::new(),
        }
    }
    pub fn resolve(&mut self, name: &str) -> Option<(String, bool)> {
        if let Some(value) = self.cache.get(name) {
            return value.clone();
        }
        if self.stack.len() >= 64 || !self.stack.insert(name.to_owned()) {
            return None;
        }
        let value = self
            .vars
            .get(name)
            .cloned()
            .and_then(|value| self.text(&value, 0))
            .map(|(value, used)| (value, used || self.overrides.contains(name)));
        self.stack.remove(name);
        self.cache.insert(name.to_owned(), value.clone());
        value
    }
    fn text(&mut self, value: &str, depth: usize) -> Option<(String, bool)> {
        if depth >= 64 {
            return None;
        }
        let mut rest = value;
        let mut output = String::new();
        let mut used = false;
        while let Some(index) = rest.find("var(") {
            output.push_str(&rest[..index]);
            let end = closing(rest, index + 3, b'(', b')')?;
            let args = split_top(&rest[index + 4..end], ',');
            let name = args.first()?.trim();
            if self.stack.contains(name) {
                return None;
            }
            let resolved = self.resolve(name).or_else(|| {
                let fallback = args.get(1..)?.join(",");
                (!fallback.is_empty())
                    .then(|| self.text(&fallback, depth + 1))
                    .flatten()
            })?;
            output.push_str(&resolved.0);
            used |= resolved.1;
            if output.len() > 65536 {
                return None;
            }
            rest = &rest[end + 1..];
        }
        output.push_str(rest);
        (output.len() <= 65536).then_some((output, used))
    }
}

#[derive(Clone, Copy)]
struct Number {
    value: f64,
    unit: &'static str,
}
struct Expression<'a> {
    text: &'a [u8],
    at: usize,
    depth: usize,
}
impl Expression<'_> {
    fn whitespace(&mut self) {
        while self.text.get(self.at).is_some_and(u8::is_ascii_whitespace) {
            self.at += 1;
        }
    }
    fn add(&mut self) -> Option<Number> {
        let mut left = self.multiply()?;
        loop {
            self.whitespace();
            let op = self.text.get(self.at).copied();
            if !matches!(op, Some(b'+' | b'-')) {
                return Some(left);
            }
            self.at += 1;
            let right = self.multiply()?;
            if left.unit != right.unit {
                return None;
            }
            left.value += if op == Some(b'+') {
                right.value
            } else {
                -right.value
            };
        }
    }
    fn multiply(&mut self) -> Option<Number> {
        let mut left = self.factor()?;
        loop {
            self.whitespace();
            let op = self.text.get(self.at).copied();
            if !matches!(op, Some(b'*' | b'/')) {
                return Some(left);
            }
            self.at += 1;
            let right = self.factor()?;
            if op == Some(b'*') {
                if !left.unit.is_empty() && !right.unit.is_empty() {
                    return None;
                }
                left.value *= right.value;
                if left.unit.is_empty() {
                    left.unit = right.unit;
                }
            } else {
                if !right.unit.is_empty() || right.value == 0. {
                    return None;
                }
                left.value /= right.value;
            }
        }
    }
    fn factor(&mut self) -> Option<Number> {
        self.whitespace();
        if self.text.get(self.at) == Some(&b'(') {
            if self.depth >= 64 {
                return None;
            }
            self.depth += 1;
            self.at += 1;
            let value = self.add()?;
            self.whitespace();
            if self.text.get(self.at) != Some(&b')') {
                return None;
            }
            self.at += 1;
            self.depth -= 1;
            return Some(value);
        }
        let start = self.at;
        if matches!(self.text.get(self.at), Some(b'+' | b'-')) {
            self.at += 1;
        }
        while self
            .text
            .get(self.at)
            .is_some_and(|ch| ch.is_ascii_digit() || *ch == b'.')
        {
            self.at += 1;
        }
        let value = std::str::from_utf8(&self.text[start..self.at])
            .ok()?
            .parse::<f64>()
            .ok()?;
        let unit = if self.text.get(self.at) == Some(&b'%') {
            self.at += 1;
            "%"
        } else if self.text.get(self.at..self.at + 3) == Some(b"deg") {
            self.at += 3;
            "deg"
        } else {
            ""
        };
        Some(Number { value, unit })
    }
}

fn calculations(text: &str) -> Result<String> {
    let mut output = text.to_owned();
    for _ in 0..64 {
        let Some(index) = output.rfind("calc(") else {
            return Ok(output);
        };
        let end =
            closing(&output, index + 4, b'(', b')').ok_or_else(|| anyhow!("Unclosed calc"))?;
        let mut parser = Expression {
            text: &output.as_bytes()[index + 5..end],
            at: 0,
            depth: 0,
        };
        let value = parser
            .add()
            .ok_or_else(|| anyhow!("Unsupported color calculation"))?;
        parser.whitespace();
        ensure!(
            parser.at == parser.text.len() && value.value.is_finite(),
            "Invalid color calculation"
        );
        let replacement = format!("{}{}", value.value, value.unit);
        output.replace_range(index..=end, &replacement);
    }
    Err(anyhow!("Color calculation nesting exceeds 64"))
}

fn weighted(text: &str) -> Result<(&str, Option<f32>)> {
    let text = text.trim();
    if text.ends_with('%')
        && let Some(index) = text.rfind(char::is_whitespace)
    {
        let value = text[index..].trim().trim_end_matches('%').parse::<f32>()? / 100.;
        ensure!(
            value.is_finite() && (0.0..=1.).contains(&value),
            "Invalid color mix weight"
        );
        return Ok((text[..index].trim(), Some(value)));
    }
    Ok((text, None))
}

fn dynamic(text: &str, depth: usize) -> Result<DynamicColor> {
    ensure!(depth < 32, "Color mix nesting exceeds 32");
    let text = text.trim();
    if let Some(inner) = text
        .strip_prefix("color-mix(")
        .and_then(|text| text.strip_suffix(')'))
    {
        let args = split_top(inner, ',');
        ensure!(args.len() == 3, "Invalid color mix");
        let mut space = args[0].split_whitespace();
        ensure!(space.next() == Some("in"), "Missing color mix space");
        let tag = match space.next() {
            Some("srgb") => ColorSpaceTag::Srgb,
            Some("srgb-linear") => ColorSpaceTag::LinearSrgb,
            Some("oklab") => ColorSpaceTag::Oklab,
            Some("oklch") => ColorSpaceTag::Oklch,
            Some("hsl") => ColorSpaceTag::Hsl,
            Some("lab") => ColorSpaceTag::Lab,
            Some("lch") => ColorSpaceTag::Lch,
            _ => return Err(anyhow!("Unsupported color mix space")),
        };
        let direction = match (space.next(), space.next(), space.next()) {
            (None, None, None) | (Some("shorter"), Some("hue"), None) => HueDirection::Shorter,
            (Some("longer"), Some("hue"), None) => HueDirection::Longer,
            (Some("increasing"), Some("hue"), None) => HueDirection::Increasing,
            (Some("decreasing"), Some("hue"), None) => HueDirection::Decreasing,
            _ => return Err(anyhow!("Unsupported color mix hue direction")),
        };
        let (left, lw) = weighted(args[1])?;
        let (right, rw) = weighted(args[2])?;
        let (lw, rw) = match (lw, rw) {
            (None, None) => (0.5, 0.5),
            (Some(l), None) => (l, 1. - l),
            (None, Some(r)) => (1. - r, r),
            (Some(l), Some(r)) => (l, r),
        };
        ensure!(lw + rw > 0., "Empty color mix");
        let mut result = dynamic(left, depth + 1)?
            .interpolate(dynamic(right, depth + 1)?, tag, direction)
            .eval(rw / (lw + rw));
        result.components[3] *= (lw + rw).min(1.);
        return Ok(result);
    }
    color::parse_color(text).map_err(|error| anyhow!("Unsupported CSS color: {error}"))
}

pub(super) fn color(text: &str) -> Result<CssColor> {
    let color = dynamic(&calculations(text)?, 0)?.convert(ColorSpaceTag::Srgb);
    ensure!(
        color.components.iter().all(|value| value.is_finite()),
        "Non-finite CSS color"
    );
    Ok(CssColor(color.components.map(|value| value.clamp(0., 1.))))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_fallbacks_cycles_calculations_and_modern_colors() {
        let vars = BTreeMap::from([
            ("--l".into(), "90%".into()),
            ("--base".into(), "hsl(0,0%,calc(var(--l) - 40%))".into()),
            ("--cycle".into(), "var(--cycle)".into()),
            ("--valid".into(), "var(--missing, var(--base))".into()),
        ]);
        let overrides = BTreeSet::from(["--l".into()]);
        let mut resolver = Resolver::new(&vars, &overrides);
        let (resolved, used) = resolver.resolve("--valid").unwrap();
        assert!(used);
        assert!((color(&resolved).unwrap().0[0] - 0.5).abs() < 0.001);
        assert!(resolver.resolve("--cycle").is_none());
        let mixed = color("color-mix(in oklch, #8b5cf6 20%, transparent)").unwrap();
        assert!((mixed.0[3] - 0.2).abs() < 0.001);
        assert!(color("oklch(0.7 0.1 240)").is_ok());
        assert!(color("hsl(0 0% calc(1% / 0))").is_err());
        assert!(color("url(https://example.invalid/color)").is_err());
    }
}
