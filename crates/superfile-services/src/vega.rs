//! Offline static Vega marks. Reject unknown semantics so the reader can show the source.
use anyhow::{Context, Result, bail, ensure};
use quick_xml::escape::escape;
use serde_json::{Map, Value};
use std::{collections::BTreeMap, fmt::Write};
use superfile_core::reader_palette::ReaderPalette;

fn object<'a>(value: &'a Value, keys: &[&str]) -> Result<&'a Map<String, Value>> {
    let map = value.as_object().context("Expected a Vega object")?;
    for key in map.keys() {
        ensure!(
            keys.contains(&key.as_str()),
            "Unsupported Vega property: {key}"
        );
    }
    Ok(map)
}

fn array<'a>(parent: &'a Value, key: &str) -> Result<&'a [Value]> {
    match parent.get(key) {
        Some(v) => Ok(v.as_array().context("Expected a Vega array")?.as_slice()),
        None => Ok(&[]),
    }
}

enum Scale {
    Linear {
        domain: [f64; 2],
        range: [f64; 2],
    },
    Band {
        domain: Vec<Value>,
        start: f64,
        step: f64,
        width: f64,
    },
}

impl Scale {
    fn position(&self, value: &Value) -> Result<f64> {
        match self {
            Self::Linear {
                domain: [a, b],
                range: [c, d],
            } => {
                let v = value.as_f64().context("Expected numeric scale input")?;
                Ok(c + if a == b { 0.5 } else { (v - a) / (b - a) } * (d - c))
            }
            Self::Band {
                domain,
                start,
                step,
                ..
            } => {
                let i = domain
                    .iter()
                    .position(|v| v == value)
                    .context("Unknown band category")?;
                Ok(start + *step * i as f64)
            }
        }
    }
}

fn scale(spec: &Value, data: &BTreeMap<&str, &[Value]>, w: f64, h: f64) -> Result<Scale> {
    object(
        spec,
        &["name", "type", "domain", "range", "zero", "padding"],
    )?;
    let range = match &spec["range"] {
        Value::String(s) if s == "width" => [0., w],
        Value::String(s) if s == "height" => [h, 0.],
        Value::Array(a) if a.len() == 2 => [
            a[0].as_f64().context("Invalid range")?,
            a[1].as_f64().context("Invalid range")?,
        ],
        _ => bail!("Unsupported Vega scale range"),
    };
    let domain = &spec["domain"];
    let inferred = domain.is_object();
    let values: Vec<Value> = if inferred {
        object(domain, &["data", "field"])?;
        let rows = data
            .get(domain["data"].as_str().context("Missing domain data")?)
            .context("Unknown domain data")?;
        let field = domain["field"].as_str().context("Missing domain field")?;
        rows.iter()
            .map(|v| v.get(field).cloned().context("Missing domain value"))
            .collect::<Result<_>>()?
    } else {
        domain
            .as_array()
            .context("Unsupported Vega domain")?
            .clone()
    };
    match spec["type"].as_str().unwrap_or("linear") {
        "linear" => {
            ensure!(
                spec.get("padding").is_none(),
                "Unsupported linear scale padding"
            );
            let values = values
                .iter()
                .map(|v| v.as_f64().context("Invalid numeric domain"))
                .collect::<Result<Vec<_>>>()?;
            ensure!(!values.is_empty(), "Empty linear domain");
            let (mut a, mut b) = if inferred {
                values
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
                        (a.min(v), b.max(v))
                    })
            } else {
                ensure!(values.len() == 2, "Unsupported piecewise domain");
                (values[0], values[1])
            };
            if spec["zero"].as_bool().unwrap_or(true) {
                if a <= b {
                    a = a.min(0.);
                    b = b.max(0.);
                } else {
                    a = a.max(0.);
                    b = b.min(0.);
                }
            }
            Ok(Scale::Linear {
                domain: [a, b],
                range,
            })
        }
        "band" => {
            let mut seen = std::collections::HashSet::new();
            let domain: Vec<_> = values
                .into_iter()
                .filter(|v| seen.insert(v.to_string()))
                .collect();
            ensure!(
                !domain.is_empty() && domain.len() <= 4096,
                "Invalid band domain"
            );
            let pad = spec["padding"].as_f64().unwrap_or(0.);
            ensure!((0. ..=1.).contains(&pad), "Invalid band padding");
            let step = (range[1] - range[0]) / (domain.len() as f64 + pad).max(1.);
            let width = step.abs() * (1. - pad);
            Ok(Scale::Band {
                domain,
                start: range[0] + step * pad - if step < 0. { width } else { 0. },
                step,
                width,
            })
        }
        other => bail!("Unsupported Vega scale: {other}"),
    }
}

fn value(
    channel: &Value,
    row: &Value,
    scales: &BTreeMap<&str, Scale>,
    width: f64,
    height: f64,
) -> Result<Value> {
    object(
        channel,
        &["value", "field", "scale", "band", "offset", "signal"],
    )?;
    if let Some(signal) = channel["signal"].as_str() {
        ensure!(
            channel.as_object().is_some_and(|m| m.len() == 1),
            "Unsupported signal modifier"
        );
        return Ok(Value::from(match signal {
            "width" => width,
            "height" => height,
            _ => bail!("Unsupported Vega signal: {signal}"),
        }));
    }
    let v = if let Some(f) = channel["field"].as_str() {
        row.get(f).context("Missing mark field")?
    } else {
        &channel["value"]
    };
    let mut n = if let Some(name) = channel["scale"].as_str() {
        let s = scales.get(name).context("Unknown mark scale")?;
        let band = match s {
            Scale::Band { width, .. } => *width,
            _ => 0.,
        };
        let base = if v.is_null() {
            ensure!(channel.get("band").is_some(), "Missing scale input");
            0.
        } else {
            s.position(v)?
        };
        base + channel["band"].as_f64().unwrap_or(0.) * band
    } else if let Some(v) = v.as_f64() {
        v
    } else {
        return Ok(v.clone());
    };
    n += channel["offset"].as_f64().unwrap_or(0.);
    ensure!(n.is_finite(), "Invalid Vega coordinate");
    Ok(n.into())
}

fn paint(v: &str) -> Result<&str> {
    ensure!(
        v.len() <= 40
            && v.bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"#.,()% -".contains(&c))
            && !v.to_ascii_lowercase().contains("url"),
        "Unsupported Vega color"
    );
    Ok(v)
}

fn draw_axis(
    out: &mut String,
    axis: &Value,
    scales: &BTreeMap<&str, Scale>,
    ink: &str,
    width: f64,
    height: f64,
) -> Result<()> {
    object(axis, &["orient", "scale", "values", "tickCount"])?;
    let scale = scales
        .get(axis["scale"].as_str().context("Missing axis scale")?)
        .context("Unknown axis scale")?;
    let bottom = match axis["orient"].as_str() {
        Some("bottom") => true,
        Some("left") => false,
        _ => bail!("Unsupported Vega axis orientation"),
    };
    let ticks = if let Some(values) = axis.get("values") {
        values.as_array().context("Invalid axis values")?.clone()
    } else {
        match scale {
            Scale::Band { domain, .. } => domain.clone(),
            Scale::Linear { domain: [a, b], .. } => {
                let count = axis["tickCount"].as_u64().unwrap_or(5).clamp(1, 50);
                (0..=count)
                    .map(|n| Value::from(a + (b - a) * n as f64 / count as f64))
                    .collect()
            }
        }
    };
    ensure!(ticks.len() <= 1024, "Too many Vega axis labels");
    let baseline = if bottom {
        format!("M0,{height}h{width}")
    } else {
        format!("M0,0v{height}")
    };
    write!(out, r#"<path d="{baseline}" stroke="{ink}" fill="none"/>"#)?;
    for tick in ticks {
        let mut pos = scale.position(&tick)?;
        if let Scale::Band { width, .. } = scale {
            pos += width / 2.;
        }
        let label = if let Some(v) = tick.as_str() {
            v.to_owned()
        } else if let Some(v) = tick.as_f64() {
            v.to_string()
        } else {
            tick.to_string()
        };
        let (x, y, anchor) = if bottom {
            (pos, height + 15., "middle")
        } else {
            (-6., pos + 3., "end")
        };
        write!(
            out,
            r#"<text x="{x}" y="{y}" text-anchor="{anchor}" fill="{ink}" font-size="10">{}</text>"#,
            escape(label)
        )?;
    }
    Ok(())
}

pub fn render(source: &str, palette: Option<ReaderPalette>) -> Result<String> {
    let spec: Value = serde_json::from_str(source)?;
    object(
        &spec,
        &[
            "$schema",
            "description",
            "width",
            "height",
            "padding",
            "data",
            "scales",
            "axes",
            "marks",
            "background",
        ],
    )?;
    let w = spec["width"].as_f64().unwrap_or(200.);
    let h = spec["height"].as_f64().unwrap_or(200.);
    let pad = spec
        .get("padding")
        .map(|v| v.as_f64().context("Unsupported Vega padding"))
        .transpose()?
        .unwrap_or(0.);
    ensure!(
        (0. ..=8192.).contains(&pad) && (1. ..=8192.).contains(&w) && (1. ..=8192.).contains(&h),
        "Invalid Vega dimensions"
    );
    let accent = palette
        .map(|p| ReaderPalette::hex(p.accent))
        .unwrap_or_else(|| "#4c78a8".into());
    let ink = palette
        .map(|p| ReaderPalette::hex(p.ink))
        .unwrap_or_else(|| "#111111".into());
    let axes = array(&spec, "axes")?;
    let left = if axes.iter().any(|a| a["orient"] == "left") {
        48.
    } else {
        0.
    };
    let bottom = if axes.iter().any(|a| a["orient"] == "bottom") {
        22.
    } else {
        0.
    };
    let mut out = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}">"#,
        w + 2. * pad + left,
        h + 2. * pad + bottom
    );
    if let Some(bg) = spec["background"].as_str() {
        let color = paint(bg)?;
        write!(out, r#"<rect width="100%" height="100%" fill="{color}"/>"#)?;
    }
    write!(out, r#"<g transform="translate({} {pad})">"#, pad + left)?;
    let mut data = BTreeMap::new();
    for item in array(&spec, "data")? {
        object(item, &["name", "values"])?;
        let rows = item["values"]
            .as_array()
            .context("Vega requires inline values")?;
        ensure!(rows.len() <= 8192, "Vega data exceeds the preview budget");
        data.insert(
            item["name"].as_str().context("Missing dataset name")?,
            rows.as_slice(),
        );
    }
    let mut scales = BTreeMap::new();
    for s in array(&spec, "scales")? {
        scales.insert(
            s["name"].as_str().context("Missing scale name")?,
            scale(s, &data, w, h)?,
        );
    }
    for axis in axes {
        draw_axis(&mut out, axis, &scales, &ink, w, h)?;
    }
    let mut count = 0;
    ensure!(spec.get("marks").is_some(), "Missing Vega marks");
    for mark in array(&spec, "marks")? {
        object(mark, &["type", "name", "from", "encode"])?;
        let kind = mark["type"].as_str().context("Missing mark type")?;
        let singleton = [Value::Null];
        let rows = if let Some(from) = mark.get("from") {
            object(from, &["data"])?;
            *data
                .get(from["data"].as_str().context("Missing mark data")?)
                .context("Unknown mark data")?
        } else {
            &singleton
        };
        count += rows.len();
        ensure!(count <= 8192, "Too many Vega marks");
        object(&mark["encode"], &["enter", "update"])?;
        let mut channels = BTreeMap::new();
        for phase in ["enter", "update"] {
            if let Some(enc) = mark["encode"].get(phase) {
                channels.extend(
                    object(
                        enc,
                        &[
                            "x",
                            "x2",
                            "y",
                            "y2",
                            "width",
                            "height",
                            "fill",
                            "stroke",
                            "strokeWidth",
                            "size",
                            "text",
                            "fontSize",
                            "shape",
                        ],
                    )?
                    .iter(),
                );
            }
        }
        for row in rows {
            let mapped = channels
                .iter()
                .map(|(k, v)| Ok((k.as_str(), value(v, row, &scales, w, h)?)))
                .collect::<Result<BTreeMap<_, _>>>()?;
            let num = |k: &str, d: f64| -> Result<f64> {
                mapped
                    .get(k)
                    .map(|v| v.as_f64().context("Expected numeric mark property"))
                    .transpose()
                    .map(|v| v.unwrap_or(d))
            };
            let string = |k: &str| mapped.get(k).and_then(|v| v.as_str());
            let x = num("x", 0.)?;
            let y = num("y", 0.)?;
            let fill =
                paint(string("fill").unwrap_or(if kind == "text" { &ink } else { &accent }))?;
            let stroke =
                paint(string("stroke").unwrap_or(if kind == "rule" { &ink } else { "none" }))?;
            let sw = num("strokeWidth", 1.)?;
            match kind {
                "rect" => {
                    let x2 = num("x2", x + num("width", 0.)?)?;
                    let y2 = num("y2", y + num("height", 0.)?)?;
                    write!(
                        out,
                        r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{fill}" stroke="{stroke}" stroke-width="{sw}"/>"#,
                        x.min(x2),
                        y.min(y2),
                        (x - x2).abs(),
                        (y - y2).abs()
                    )?;
                }
                "rule" => {
                    write!(
                        out,
                        r#"<line x1="{x}" y1="{y}" x2="{}" y2="{}" stroke="{stroke}" stroke-width="{sw}"/>"#,
                        num("x2", x)?,
                        num("y2", y)?
                    )?;
                }
                "symbol" => {
                    ensure!(
                        string("shape").is_none_or(|s| s == "circle"),
                        "Unsupported Vega symbol shape"
                    );
                    write!(
                        out,
                        r#"<circle cx="{x}" cy="{y}" r="{}" fill="{fill}" stroke="{stroke}" stroke-width="{sw}"/>"#,
                        num("size", 64.)?.max(0.).sqrt() / 2.
                    )?;
                }
                "text" => {
                    let label = mapped
                        .get("text")
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| v.to_string())
                        })
                        .unwrap_or_default();
                    write!(
                        out,
                        r#"<text x="{x}" y="{y}" fill="{fill}" font-size="{}">{}</text>"#,
                        num("fontSize", 11.)?,
                        escape(label)
                    )?;
                }
                _ => bail!("Unsupported Vega mark: {kind}"),
            }
        }
    }
    out.push_str("</g></svg>");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projects_inline_values_and_escapes_labels() {
        let spec = serde_json::json!({"width":100,"height":40,"data":[{"name":"rows","values":[{"v":7,"label":"A&B"}]}],
            "scales":[{"name":"x","domain":[0,10],"range":"width"}],"marks":[{"type":"text","from":{"data":"rows"},
            "encode":{"update":{"x":{"scale":"x","field":"v"},"y":{"value":25},"text":{"field":"label"}}}}]});
        let palette = ReaderPalette {
            paper: [0; 3],
            ink: [1, 2, 3],
            surface: [2; 3],
            accent: [9; 3],
        };
        let svg = render(&spec.to_string(), Some(palette)).unwrap();
        assert!(svg.contains(r##"x="70" y="25" fill="#010203""##));
        assert!(svg.contains("A&amp;B"));
        assert!(render(r#"{"data":[{"name":"x","url":"file:///missing"}]}"#, None).is_err());
    }
}
