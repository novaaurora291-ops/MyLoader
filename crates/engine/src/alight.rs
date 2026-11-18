//! Alight Motion `.xml` project / preset importer.
//!
//! Format reference (reverse engineered, AM 5.x):
//! * root `<scene width height fps totalTime(ms) bgcolor="#AARRGGBB">`
//! * elements in z-order (first = back): `<shape>`, `<embedScene>`, `<text>`,
//!   `<drawing>`, `<audio>`, `<camera>`, `<nullobj>`
//! * common attributes: `label`, `hidden`, `startTime`/`endTime` (ms),
//!   `blending` (omitted = normal), `fillType`
//! * `<transform>` children are keyables: `<location value="x,y,z">`,
//!   `<scale value="sx,sy">`, `<rotation value>`, `<opacity value>` or `<kf t v>`
//! * a `.rect` shape is 100x100 units unless `<property name="size">` says otherwise
//! * an **adjustment layer** is a shape whose first effect is "Copy Background"
//!   (`com.alightcreative.effects.lift`, `fill` = 0); the grading effects follow.
//!
//! What maps onto the engine:
//! * adjustment shapes            -> [`LayerKind::Adjustment`] with [`CcParams`]
//! * solid colour shapes          -> [`LayerKind::Rectangle`] (circles get a full corner radius)
//! * blend mode / opacity / time  -> layer fields
//! * unknown effects              -> `raw_effects` (+ a warning) so nothing is silently lost

use crate::model::*;
use anyhow::{anyhow, Context, Result};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PresetStats {
    pub elements: usize,
    pub adjustments: usize,
    pub rectangles: usize,
    pub skipped: usize,
    pub effects_total: usize,
    pub effects_mapped: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedPreset {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// seconds
    pub duration: f64,
    pub background: [f32; 4],
    /// Bottom -> top, in the preset's own pixel space.
    pub layers: Vec<Layer>,
    pub warnings: Vec<String>,
    pub stats: PresetStats,
}

pub fn parse_preset_file(path: &std::path::Path) -> Result<ParsedPreset> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    // AM files are UTF-8; tolerate a BOM / stray bytes.
    let text = String::from_utf8_lossy(&bytes);
    parse_preset_str(text.trim_start_matches('\u{feff}'))
}

pub fn parse_preset_str(xml: &str) -> Result<ParsedPreset> {
    let doc = Document::parse(xml).map_err(|e| anyhow!("invalid XML: {e}"))?;
    let root = doc.root_element();
    let scene = if root.has_tag_name("scene") {
        root
    } else {
        root.descendants()
            .find(|n| n.has_tag_name("scene"))
            .ok_or_else(|| anyhow!("no <scene> element found - is this an Alight Motion XML?"))?
    };

    let width = attr_f(scene, "width").unwrap_or(1080.0).max(1.0) as u32;
    let height = attr_f(scene, "height").unwrap_or(1920.0).max(1.0) as u32;
    let fps = attr_f(scene, "fps").unwrap_or(30.0);
    let duration = attr_f(scene, "totalTime").unwrap_or(0.0) / 1000.0;
    let background = scene.attribute("bgcolor").and_then(parse_argb).unwrap_or([0.0, 0.0, 0.0, 1.0]);

    let mut ctx = Ctx {
        short_edge: width.min(height) as f32,
        layers: vec![],
        warnings: vec![],
        stats: PresetStats::default(),
        ids: HashSet::new(),
    };
    walk_scene(&mut ctx, scene, 1.0, 0.0);

    Ok(ParsedPreset {
        title: scene.attribute("title").unwrap_or("Alight Motion preset").to_string(),
        width,
        height,
        fps,
        duration,
        background,
        layers: ctx.layers,
        warnings: ctx.warnings,
        stats: ctx.stats,
    })
}

/// Append the preset's layers on top of `project`, rescaling them from the
/// preset's canvas to the project's composition. Returns the ids added.
pub fn import_into(project: &mut Project, preset: &ParsedPreset) -> Vec<String> {
    let sx = project.composition.width as f32 / preset.width.max(1) as f32;
    let sy = project.composition.height as f32 / preset.height.max(1) as f32;
    let existing: HashSet<String> = project.layers.iter().map(|l| l.id.clone()).collect();
    let mut added = vec![];
    for (i, l) in preset.layers.iter().enumerate() {
        let mut l = l.clone();
        l.transform.x *= sx;
        l.transform.y *= sy;
        l.transform.width *= sx;
        l.transform.height *= sy;
        l.cc.blur *= sx.min(sy);
        if let LayerKind::Rectangle { corner_radius, .. } = &mut l.kind {
            *corner_radius *= sx.min(sy);
        }
        let mut id = l.id.clone();
        let mut n = i;
        while existing.contains(&id) || added.contains(&id) {
            n += 1;
            id = format!("{}-{}", l.id, n);
        }
        l.id = id.clone();
        added.push(id);
        project.layers.push(l);
    }
    added
}

struct Ctx {
    short_edge: f32,
    layers: Vec<Layer>,
    warnings: Vec<String>,
    stats: PresetStats,
    ids: HashSet<String>,
}

fn walk_scene(ctx: &mut Ctx, scene: Node, parent_opacity: f32, time_offset: f64) {
    for el in scene.children().filter(|n| n.is_element()) {
        let tag = el.tag_name().name();
        match tag {
            "media" | "bookmark" | "audio" | "camera" | "nullobj" | "speedMap" => {}
            "shape" => {
                ctx.stats.elements += 1;
                shape(ctx, el, parent_opacity, time_offset);
            }
            "embedScene" | "group" => {
                ctx.stats.elements += 1;
                if attr_bool(el, "hidden") {
                    continue;
                }
                // Nested scenes are flattened; their transform is assumed to be
                // centred / identity (the common case in CC presets).
                let op = transform_value(el, "opacity").map(|v| v[0] as f32).unwrap_or(1.0);
                let start = attr_f(el, "startTime").unwrap_or(0.0) / 1000.0;
                if let Some(inner) = el.children().find(|n| n.has_tag_name("scene")) {
                    walk_scene(ctx, inner, parent_opacity * normalise_opacity(op), time_offset + start);
                } else {
                    walk_scene(ctx, el, parent_opacity * normalise_opacity(op), time_offset + start);
                }
                ctx.warnings.push(format!(
                    "Nested scene '{}' was flattened (its own transform is ignored)",
                    el.attribute("label").unwrap_or("group")
                ));
            }
            "text" | "drawing" => {
                ctx.stats.elements += 1;
                ctx.stats.skipped += 1;
                ctx.warnings.push(format!(
                    "Skipped {tag} layer '{}' (only video, adjustment and rectangle layers are supported)",
                    el.attribute("label").unwrap_or(tag)
                ));
            }
            _ => {}
        }
    }
}

fn shape(ctx: &mut Ctx, el: Node, parent_opacity: f32, time_offset: f64) {
    let label = el.attribute("label").unwrap_or("Layer").to_string();
    if attr_bool(el, "hidden") {
        ctx.stats.skipped += 1;
        ctx.warnings.push(format!("Skipped hidden layer '{label}'"));
        return;
    }
    let fill_type = el.attribute("fillType").unwrap_or("color");
    let shape_kind = el.attribute("s").unwrap_or(".rect").trim_start_matches('.').to_string();

    // ---- transform -------------------------------------------------------
    let loc = transform_value(el, "location").unwrap_or_else(|| vec![0.0, 0.0]);
    let scale = transform_value(el, "scale").unwrap_or_else(|| vec![1.0, 1.0]);
    let rotation = transform_value(el, "rotation").map(|v| v[0]).unwrap_or(0.0) as f32;
    let opacity = normalise_opacity(transform_value(el, "opacity").map(|v| v[0] as f32).unwrap_or(1.0));
    let size = direct_property(el, "size").and_then(|p| keyable_value(p)).unwrap_or_else(|| vec![100.0, 100.0]);
    let (sw, sh) = (size[0], *size.get(1).unwrap_or(&size[0]));
    let (scx, scy) = (scale[0], *scale.get(1).unwrap_or(&scale[0]));
    let transform = Transform {
        x: loc[0] as f32,
        y: *loc.get(1).unwrap_or(&0.0) as f32,
        width: (sw * scx).abs() as f32,
        height: (sh * scy).abs() as f32,
        rotation,
    };

    let blend = match el.attribute("blending").or_else(|| child_value(el, "blendMode")) {
        Some(b) => BlendMode::from_loose(b).unwrap_or_else(|| {
            ctx.warnings.push(format!("Layer '{label}': blend mode '{b}' not supported, using Normal"));
            BlendMode::Normal
        }),
        None => BlendMode::Normal,
    };

    let start = attr_f(el, "startTime").map(|v| v / 1000.0).unwrap_or(0.0) + time_offset;
    let end = attr_f(el, "endTime").map(|v| v / 1000.0 + time_offset);

    // ---- effects -----------------------------------------------------------
    let effects: Vec<Node> = el.children().filter(|n| n.has_tag_name("effect") && !attr_bool(*n, "hidden")).collect();
    let is_adjustment = effects.iter().any(|e| {
        effect_short_id(e) == "lift" && effect_props(*e).get("fill").map_or(0.0, |v| v[0]) < 0.5
    });

    let mut cc = CcParams::default();
    let mut raw = vec![];
    for e in &effects {
        ctx.stats.effects_total += 1;
        let props = effect_props(*e);
        let short = effect_short_id(e);
        if short == "lift" {
            ctx.stats.effects_mapped += 1;
            continue;
        }
        let mapped = is_adjustment && map_effect(&mut cc, &short, &props, ctx.short_edge, &label, &mut ctx.warnings);
        if mapped {
            ctx.stats.effects_mapped += 1;
        } else {
            raw.push(RawEffect {
                id: e.attribute("id").unwrap_or("?").to_string(),
                params: props.iter().map(|(k, v)| (k.clone(), fmt_vec(v))).collect::<BTreeMap<_, _>>(),
            });
            ctx.warnings.push(format!(
                "Layer '{label}': effect '{}' is not supported yet and was kept as metadata only",
                e.attribute("id").unwrap_or("?")
            ));
        }
    }

    let fade = if is_adjustment { None } else { gradient_bottom_fade(el) };
    let kind = if is_adjustment {
        ctx.stats.adjustments += 1;
        LayerKind::Adjustment
    } else if let Some(strength) = fade {
        // Vertical transparent -> black gradient = "bottom black fade" for lyrics.
        ctx.stats.adjustments += 1;
        cc.bottom_fade = strength;
        cc.bottom_fade_height = 1.0;
        LayerKind::Adjustment
    } else if fill_type == "media" || el.attribute("fillImage").is_some() || el.attribute("fillVideo").is_some() {
        ctx.stats.skipped += 1;
        ctx.warnings.push(format!(
            "Skipped media placeholder '{label}' - import your own video as the bottom layer instead"
        ));
        return;
    } else {
        ctx.stats.rectangles += 1;
        let color = child_value(el, "fillColor").and_then(parse_argb).unwrap_or([1.0, 1.0, 1.0, 1.0]);
        let mut corner = direct_property(el, "cornerRadius").and_then(keyable_value).map(|v| v[0] as f32).unwrap_or(0.0);
        corner *= scx.abs().min(scy.abs()) as f32;
        match shape_kind.as_str() {
            "rect" | "roundrect" | "roundedrect" => {}
            "circle" | "ellipse" => corner = transform.width.min(transform.height) / 2.0,
            other => ctx.warnings.push(format!("Layer '{label}': shape '.{other}' approximated as a rectangle")),
        }
        LayerKind::Rectangle { color, corner_radius: corner }
    };

    let mut id = format!("am-{}", el.attribute("id").unwrap_or("0"));
    while ctx.ids.contains(&id) {
        id.push('x');
    }
    ctx.ids.insert(id.clone());

    ctx.layers.push(Layer {
        id,
        name: label,
        kind,
        visible: true,
        locked: false,
        opacity: (opacity * parent_opacity).clamp(0.0, 1.0),
        blend,
        transform,
        cc: cc.clamped(),
        start,
        end,
        raw_effects: raw,
    });
}

/// Map one Alight Motion effect onto the CC stack. Returns false if unknown.
fn map_effect(
    cc: &mut CcParams,
    short: &str,
    p: &BTreeMap<String, Vec<f64>>,
    short_edge: f32,
    label: &str,
    warnings: &mut Vec<String>,
) -> bool {
    let g = |k: &str, d: f64| p.get(k).map(|v| v[0]).unwrap_or(d) as f32;
    match short {
        "brightcont" | "brightcont2" => {
            cc.brightness += g("brightness", 0.0);
            cc.contrast += g("contrast", 0.0);
        }
        "coloradj" => {
            cc.brightness += g("brightness", 0.0);
            cc.contrast += g("contrast", 0.0);
            cc.saturation += g("saturation", 0.0) + (g("vib", 1.0) - 1.0) * 0.5;
            if g("hue", 0.0) != 0.0 {
                warnings.push(format!("Layer '{label}': hue rotation in Color Adjust is ignored"));
            }
        }
        "exposure" => {
            cc.exposure += g("exposure", 0.0);
            cc.brightness += g("offset", 0.0);
            let gamma = g("gamma", 1.0);
            if (gamma - 1.0).abs() > 0.01 {
                // Approximate gamma with a contrast nudge.
                cc.contrast += (1.0 / gamma - 1.0) * 0.25;
            }
        }
        "highlightshadow" => {
            cc.highlights += g("highlights", 1.0) - 1.0;
            cc.shadows += g("shadows", 1.0) - 1.0;
        }
        "gaussianblur" | "boxblur" | "boxblur2" | "boxblur3" | "fastboxblur" | "lensblur" | "lensblur2" => {
            // AM strength 0..2 is relative to the frame; ~4% of the short edge per unit.
            let s = g("strength", if short == "gaussianblur" { 0.15 } else { g("radius", 0.15) as f64 });
            cc.blur = cc.blur.max(s * 0.04 * short_edge);
        }
        "vignette" => {
            cc.vignette = g("strength", 0.8);
            cc.vignette_softness = g("feather", 0.5);
        }
        "satvib" | "saturation" | "vibrance" => {
            let s = p.get("saturation").or_else(|| p.get("sat")).or_else(|| p.get("amount")).map(|v| v[0]).unwrap_or(0.0) as f32;
            // satvib stores saturation as a multiplier (1 = unchanged)
            cc.saturation += if short == "satvib" { s - 1.0 } else { s };
            if let Some(v) = p.get("vibrance").or_else(|| p.get("vib")) {
                cc.saturation += (v[0] as f32 - if short == "satvib" { 1.0 } else { 0.0 }) * 0.5;
            }
        }
        _ => {
            // Generic fallback: any effect exposing well-known parameter names.
            let mut hit = false;
            for (k, v) in p {
                let k = k.to_ascii_lowercase();
                let v = v[0] as f32;
                match k.as_str() {
                    "brightness" => {
                        cc.brightness += v;
                        hit = true
                    }
                    "contrast" => {
                        cc.contrast += v;
                        hit = true
                    }
                    "exposure" => {
                        cc.exposure += v;
                        hit = true
                    }
                    "saturation" => {
                        cc.saturation += v;
                        hit = true
                    }
                    _ => {}
                }
            }
            return hit;
        }
    }
    true
}

/// Detects the classic lyrics "bottom fade": a linear gradient shape that goes
/// from transparent (top) to dark (bottom). Returns the fade strength.
fn gradient_bottom_fade(el: Node) -> Option<f32> {
    let g = el.children().find(|n| n.has_tag_name("gradient"))?;
    if g.attribute("type").unwrap_or("linear") != "linear" {
        return None;
    }
    let sc = parse_argb(g.attribute("startColor").unwrap_or("#ff000000"))?;
    let ec = parse_argb(g.attribute("endColor").unwrap_or("#ffffffff"))?;
    let s = parse_vec(g.attribute("start").unwrap_or("0,0"))?;
    let e = parse_vec(g.attribute("end").unwrap_or("1,1"))?;
    let vertical = (s[0] - e.first().copied().unwrap_or(0.0)).abs() < 0.2 && e.get(1).copied().unwrap_or(1.0) > s.get(1).copied().unwrap_or(0.0);
    let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    if vertical && luma(ec) < 0.15 && ec[3] > sc[3] + 0.2 {
        Some(ec[3])
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

fn effect_short_id(e: &Node) -> String {
    e.attribute("id").unwrap_or("").rsplit('.').next().unwrap_or("").to_ascii_lowercase()
}

fn effect_props(e: Node) -> BTreeMap<String, Vec<f64>> {
    let mut m = BTreeMap::new();
    for p in e.children().filter(|n| n.has_tag_name("property") || n.has_tag_name("param")) {
        let name = p.attribute("name").or_else(|| p.attribute("id")).unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        if let Some(v) = keyable_value(p) {
            m.insert(name, v);
        }
    }
    m
}

fn direct_property<'a, 'i>(el: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    el.children().find(|n| n.has_tag_name("property") && n.attribute("name") == Some(name))
}

fn child_value<'a>(el: Node<'a, '_>, tag: &str) -> Option<&'a str> {
    el.children().find(|n| n.has_tag_name(tag)).and_then(|n| {
        n.attribute("value").or_else(|| n.children().find(|k| k.has_tag_name("kf")).and_then(|k| k.attribute("v")))
    })
}

fn transform_value(el: Node, name: &str) -> Option<Vec<f64>> {
    // AM writes keyables inside <transform>; very old files put them directly on the element.
    let t = el.children().find(|n| n.has_tag_name("transform"));
    t.and_then(|t| t.children().find(|n| n.has_tag_name(name)))
        .or_else(|| el.children().find(|n| n.has_tag_name(name)))
        .and_then(keyable_value)
}

/// `value="a,b,c"` or first `<kf v="...">` (keyframed values use the first key).
fn keyable_value(n: Node) -> Option<Vec<f64>> {
    let raw = n
        .attribute("value")
        .or_else(|| n.children().find(|k| k.has_tag_name("kf")).and_then(|k| k.attribute("v")))?;
    parse_vec(raw)
}

fn parse_vec(raw: &str) -> Option<Vec<f64>> {
    match raw.trim() {
        "true" => return Some(vec![1.0]),
        "false" => return Some(vec![0.0]),
        _ => {}
    }
    let v: Vec<f64> = raw.split(',').filter_map(|s| s.trim().parse::<f64>().ok()).collect();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

fn fmt_vec(v: &[f64]) -> String {
    v.iter().map(|x| format!("{x}")).collect::<Vec<_>>().join(",")
}

fn attr_f(n: Node, name: &str) -> Option<f64> {
    n.attribute(name).and_then(|s| s.trim().parse().ok())
}

fn attr_bool(n: Node, name: &str) -> bool {
    n.attribute(name).map_or(false, |s| s.eq_ignore_ascii_case("true") || s == "1")
}

/// AM opacity is 0..1; some third-party generators write 0..100.
fn normalise_opacity(v: f32) -> f32 {
    if v > 1.0 {
        (v / 100.0).clamp(0.0, 1.0)
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// `#AARRGGBB` (Alight Motion) or `#RRGGBB` -> straight RGBA 0..1
pub fn parse_argb(s: &str) -> Option<[f32; 4]> {
    let h = s.trim().trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok().map(|b| b as f32 / 255.0);
    match h.len() {
        8 => Some([p(2)?, p(4)?, p(6)?, p(0)?]),
        6 => Some([p(0)?, p(2)?, p(4)?, 1.0]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HDR_PRESET: &str = include_str!("../../../presets/hdr-cc-lyrics.xml");

    #[test]
    fn parses_bundled_hdr_preset() {
        let p = parse_preset_str(HDR_PRESET).unwrap();
        assert_eq!((p.width, p.height), (1080, 1920));
        assert_eq!(p.fps, 30.0);
        // media placeholder skipped, 4 adjustments, 1 rectangle
        assert_eq!(p.stats.adjustments, 4, "{:#?}", p.warnings);
        assert_eq!(p.stats.rectangles, 1);
        let names: Vec<_> = p.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["HDR Tone", "Contrast Pop", "Bloom", "Lyrics Fade", "Warm Tint"]);

        let hdr = &p.layers[0];
        assert!(matches!(hdr.kind, LayerKind::Adjustment));
        assert!((hdr.cc.highlights - (-0.35)).abs() < 1e-4);
        assert!((hdr.cc.shadows - 0.4).abs() < 1e-4);
        assert!((hdr.cc.exposure - 0.25).abs() < 1e-4);
        assert!((hdr.transform.width - 1080.0).abs() < 0.01);
        assert!((hdr.transform.height - 1920.0).abs() < 0.01);

        let pop = &p.layers[1];
        assert!((pop.cc.contrast - 0.3).abs() < 1e-4);
        assert!((pop.cc.saturation - 0.25).abs() < 1e-4);

        let bloom = &p.layers[2];
        assert_eq!(bloom.blend, BlendMode::Screen);
        assert!((bloom.opacity - 0.35).abs() < 1e-4);
        assert!(bloom.cc.blur > 30.0);

        let warm = &p.layers[4];
        assert_eq!(warm.blend, BlendMode::Overlay);
        match warm.kind {
            LayerKind::Rectangle { color, .. } => assert!(color[0] > 0.9 && color[2] < 0.3),
            _ => panic!("expected rectangle"),
        }
    }

    #[test]
    fn argb_colours() {
        assert_eq!(parse_argb("#ff000000"), Some([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(parse_argb("#80FF0000").unwrap()[3], 128.0 / 255.0);
        assert_eq!(parse_argb("#00ff00"), Some([0.0, 1.0, 0.0, 1.0]));
        assert_eq!(parse_argb("nope"), None);
    }

    #[test]
    fn keyframes_and_blend_names() {
        let xml = r##"<scene width="1000" height="1000" fps="60" totalTime="2000">
          <shape id="1" label="A" startTime="500" endTime="1500" blending="multiply" s=".rect">
            <transform><location value="500,500,0"/><scale value="10,10"/>
              <opacity><kf t="0" v="0.25"/><kf t="1" v="1"/></opacity></transform>
            <fillColor value="#ff112233"/>
          </shape></scene>"##;
        let p = parse_preset_str(xml).unwrap();
        let l = &p.layers[0];
        assert_eq!(l.blend, BlendMode::Multiply);
        assert_eq!(l.opacity, 0.25);
        assert_eq!((l.start, l.end), (0.5, Some(1.5)));
        assert_eq!(l.transform.width, 1000.0);
    }

    #[test]
    fn import_rescales_to_project() {
        let p = parse_preset_str(HDR_PRESET).unwrap();
        let mut proj = Project::new(Composition { width: 1920, height: 1080, ..Default::default() });
        let ids = import_into(&mut proj, &p);
        assert_eq!(ids.len(), p.layers.len());
        let t = proj.layers[0].transform;
        assert!((t.width - 1920.0).abs() < 0.01 && (t.height - 1080.0).abs() < 0.01);
    }
}
