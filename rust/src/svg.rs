const DEFAULT_PRESET_PLUGINS: &[&str] = &[
    "removeDoctype",
    "removeXMLProcInst",
    "removeComments",
    "removeDeprecatedAttrs",
    "removeMetadata",
    "removeEditorsNSData",
    "cleanupAttrs",
    "mergeStyles",
    "inlineStyles",
    "minifyStyles",
    "cleanupIds",
    "removeUselessDefs",
    "cleanupNumericValues",
    "convertColors",
    "removeUnknownsAndDefaults",
    "removeNonInheritableGroupAttrs",
    "removeUselessStrokeAndFill",
    "cleanupEnableBackground",
    "removeHiddenElems",
    "removeEmptyText",
    "convertShapeToPath",
    "convertEllipseToCircle",
    "moveElemsAttrsToGroup",
    "moveGroupAttrsToElems",
    "collapseGroups",
    "convertPathData",
    "convertTransform",
    "removeEmptyAttrs",
    "removeEmptyContainers",
    "mergePaths",
    "removeUnusedNS",
    "sortAttrs",
    "sortDefsChildren",
    "removeDesc",
];

const EXTRA_PLUGINS: &[&str] = &[
    "addAttributesToSVGElement",
    "addClassesToSVGElement",
    "cleanupIDs",
    "cleanupListOfValues",
    "convertOneStopGradients",
    "convertStyleToAttrs",
    "prefixIds",
    "removeAttributesBySelector",
    "removeAttrs",
    "removeDimensions",
    "removeElementsByAttr",
    "removeOffCanvasPaths",
    "removeRasterImages",
    "removeScriptElement",
    "removeScripts",
    "removeEventAttributes",
    "removeStyleElement",
    "removeTitle",
    "removeUnsafeLinks",
    "removeViewBox",
    "removeXlink",
    "removeXMLNS",
    "reusePaths",
];

fn builtin_plugins() -> Vec<String> {
    let mut plugins: Vec<String> = DEFAULT_PRESET_PLUGINS.iter().map(|s| s.to_string()).collect();
    for name in EXTRA_PLUGINS {
        if !plugins.iter().any(|existing| existing == name) {
            plugins.push((*name).to_string());
        }
    }
    plugins
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PluginSpecCore {
    name: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OptimizeOptionsCore {
    #[serde(default = "default_preset")]
    preset: String,
    #[serde(default)]
    plugins: Vec<PluginSpecCore>,
    #[serde(default)]
    disabled: Vec<String>,
    #[serde(default)]
    float_precision: Option<usize>,
    #[serde(default)]
    multipass: bool,
    #[serde(default)]
    pretty: bool,
    #[serde(default = "default_indent")]
    indent: usize,
    #[serde(default)]
    eol: Option<String>,
    #[serde(default)]
    final_newline: bool,
    #[serde(default)]
    datauri: Option<String>,
}

fn default_preset() -> String {
    "default".to_string()
}

fn default_indent() -> usize {
    2
}

impl Default for OptimizeOptionsCore {
    fn default() -> Self {
        Self {
            preset: default_preset(),
            plugins: Vec::new(),
            disabled: Vec::new(),
            float_precision: None,
            multipass: false,
            pretty: false,
            indent: 2,
            eol: None,
            final_newline: false,
            datauri: None,
        }
    }
}

fn parse_opt_options_json(options_json: Option<&str>) -> Result<OptimizeOptionsCore> {
    if let Some(raw) = options_json {
        if raw.trim().is_empty() {
            return Ok(OptimizeOptionsCore::default());
        }
        let mut options: OptimizeOptionsCore = serde_json::from_str(raw)
            .map_err(|e| SvgoError(format!("Invalid optimizer options JSON: {}", e)))?;
        if let Some(p) = options.float_precision {
            options.float_precision = Some(p.min(20));
        }
        Ok(options)
    } else {
        Ok(OptimizeOptionsCore::default())
    }
}

fn effective_plugins(options: &OptimizeOptionsCore) -> Result<Vec<PluginSpecCore>> {
    let mut plugins = Vec::new();
    let disabled: HashSet<_> = options.disabled.iter().cloned().collect();
    if options.preset == "default" {
        plugins.extend(
            DEFAULT_PRESET_PLUGINS
                .iter()
                .filter(|name| !disabled.contains(**name))
                .map(|name| PluginSpecCore {
                    name: (*name).to_string(),
                    params: Value::Object(Default::default()),
                }),
        );
    } else if options.preset != "none" {
        return Err("--svgo-preset must be default or none".into());
    }
    plugins.extend(options.plugins.iter().cloned());
    Ok(plugins)
}

fn remove_doctype(mut text: String) -> String {
    loop {
        let Some(start) = lower_find(&text, "<!doctype") else {
            break;
        };
        let Some(end_rel) = text[start..].find('>') else {
            break;
        };
        text.replace_range(start..start + end_rel + 1, "");
    }
    text
}

fn remove_xml_proc(mut text: String) -> String {
    loop {
        let Some(start) = lower_find(&text, "<?xml") else {
            break;
        };
        let Some(end_rel) = text[start..].find("?>") else {
            break;
        };
        text.replace_range(start..start + end_rel + 2, "");
    }
    text
}

fn remove_comments(mut text: String) -> String {
    loop {
        let Some(start) = text.find("<!--") else {
            break;
        };
        let Some(end_rel) = text[start + 4..].find("-->") else {
            break;
        };
        text.replace_range(start..start + 4 + end_rel + 3, "");
    }
    text
}

fn lower_find(text: &str, needle: &str) -> Option<usize> {
    text.to_ascii_lowercase().find(needle)
}

fn parse_svg_element(text: &str) -> Result<Element> {
    Element::parse(Cursor::new(text.trim().as_bytes()))
        .map_err(|e| SvgoError(format!("Could not parse SVG: {}", e)))
}

fn serialize_element(root: &Element, pretty: bool, indent: usize) -> Result<String> {
    let mut out = Vec::new();
    let config = EmitterConfig::new()
        .perform_indent(pretty)
        .write_document_declaration(false)
        .indent_string(" ".repeat(indent));
    root.write_with_config(&mut out, config)
        .map_err(|e| SvgoError(format!("Could not serialize SVG: {}", e)))?;
    let mut text = String::from_utf8(out).map_err(|e| SvgoError(e.to_string()))?;
    if !pretty {
        text = compact_tag_whitespace(&text);
    }
    Ok(text.trim().to_string())
}

fn compact_tag_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '>' {
            out.push(c);
            let mut ws = String::new();
            while chars.peek().is_some_and(|n| n.is_whitespace()) {
                ws.push(chars.next().unwrap());
            }
            if chars.peek() == Some(&'<') {
                continue;
            }
            out.push_str(&ws);
        } else {
            out.push(c);
        }
    }
    out.replace(" />", "/>")
}

fn local_name(name: &str) -> &str {
    name.rsplit('}').next().unwrap_or(name).rsplit(':').next().unwrap_or(name)
}

fn walk_mut<F: FnMut(&mut Element)>(element: &mut Element, f: &mut F) {
    f(element);
    for child in &mut element.children {
        if let XMLNode::Element(child) = child {
            walk_mut(child, f);
        }
    }
}

fn walk_ref<'a>(element: &'a Element, out: &mut Vec<&'a Element>) {
    out.push(element);
    for child in &element.children {
        if let XMLNode::Element(child) = child {
            walk_ref(child, out);
        }
    }
}

fn remove_children_by<F: Fn(&Element) -> bool>(element: &mut Element, predicate: &F) {
    element.children.retain(|child| match child {
        XMLNode::Element(child_el) => !predicate(child_el),
        _ => true,
    });
    for child in &mut element.children {
        if let XMLNode::Element(child_el) = child {
            remove_children_by(child_el, predicate);
        }
    }
}

fn attr_value<'a>(element: &'a Element, name: &str) -> Option<&'a str> {
    element
        .attributes
        .iter()
        .find(|(key, _)| local_name(key) == name)
        .map(|(_, value)| value.as_str())
}

fn attr_remove(element: &mut Element, name: &str) {
    let keys: Vec<String> = element
        .attributes
        .keys()
        .filter(|key| local_name(key) == name)
        .cloned()
        .collect();
    for key in keys {
        element.attributes.remove(&key);
    }
}

fn parse_float(value: Option<&str>, default: f64) -> f64 {
    value
        .and_then(|v| parse_float_list(v).first().copied())
        .unwrap_or(default)
}

fn shape_to_path_d(element: &Element, precision: Option<usize>) -> Option<String> {
    let decimals = precision.unwrap_or(4);
    let name = local_name(&element.name);
    match name {
        "rect" => {
            let x = parse_float(attr_value(element, "x"), 0.0);
            let y = parse_float(attr_value(element, "y"), 0.0);
            let width = parse_float(attr_value(element, "width"), 0.0);
            let height = parse_float(attr_value(element, "height"), 0.0);
            if width <= 0.0 || height <= 0.0 {
                return None;
            }
            let rx = parse_float(attr_value(element, "rx"), parse_float(attr_value(element, "ry"), 0.0));
            let ry = parse_float(attr_value(element, "ry"), rx);
            if rx == 0.0 && ry == 0.0 {
                return rect_to_path_core(x, y, width, height, 0.0, Some(0.0), decimals, true)
                    .ok()
                    .and_then(|d| PathDataCore::parse(&d).ok()?.optimize(Some("closed")).ok()?.to_string(decimals, true).into());
            }
            let d = format!(
                "M{} {}H{}A{} {} 0 0 1 {} {}V{}A{} {} 0 0 1 {} {}H{}A{} {} 0 0 1 {} {}V{}A{} {} 0 0 1 {} {}Z",
                x + rx,
                y,
                x + width - rx,
                rx,
                ry,
                x + width,
                y + ry,
                y + height - ry,
                rx,
                ry,
                x + width - rx,
                y + height,
                x + rx,
                rx,
                ry,
                x,
                y + height - ry,
                y + ry,
                rx,
                ry,
                x + rx,
                y
            );
            PathDataCore::parse(&d).ok().map(|p| p.to_string(decimals, true))
        }
        "line" => {
            let d = format!(
                "M{} {}L{} {}",
                parse_float(attr_value(element, "x1"), 0.0),
                parse_float(attr_value(element, "y1"), 0.0),
                parse_float(attr_value(element, "x2"), 0.0),
                parse_float(attr_value(element, "y2"), 0.0)
            );
            PathDataCore::parse(&d).ok().map(|p| p.to_string(decimals, true))
        }
        "polyline" | "polygon" => {
            let points = parse_points(attr_value(element, "points").unwrap_or(""));
            if points.is_empty() {
                return None;
            }
            let mut d = format!(
                "M{}",
                points
                    .iter()
                    .map(|(x, y)| format!("{} {}", x, y))
                    .collect::<Vec<_>>()
                    .join("L")
            );
            if name == "polygon" {
                d.push('Z');
            }
            PathDataCore::parse(&d).ok().map(|p| p.to_string(decimals, true))
        }
        "circle" => {
            let cx = parse_float(attr_value(element, "cx"), 0.0);
            let cy = parse_float(attr_value(element, "cy"), 0.0);
            let r = parse_float(attr_value(element, "r"), 0.0);
            if r <= 0.0 {
                return None;
            }
            let d = format!("M{} {}A{} {} 0 1 0 {} {}A{} {} 0 1 0 {} {}Z", cx - r, cy, r, r, cx + r, cy, r, r, cx - r, cy);
            PathDataCore::parse(&d).ok().map(|p| p.to_string(decimals, true))
        }
        "ellipse" => {
            let cx = parse_float(attr_value(element, "cx"), 0.0);
            let cy = parse_float(attr_value(element, "cy"), 0.0);
            let rx = parse_float(attr_value(element, "rx"), 0.0);
            let ry = parse_float(attr_value(element, "ry"), 0.0);
            if rx <= 0.0 || ry <= 0.0 {
                return None;
            }
            let d = format!("M{} {}A{} {} 0 1 0 {} {}A{} {} 0 1 0 {} {}Z", cx - rx, cy, rx, ry, cx + rx, cy, rx, ry, cx - rx, cy);
            PathDataCore::parse(&d).ok().map(|p| p.to_string(decimals, true))
        }
        _ => None,
    }
}

fn convert_shapes_to_paths(root: &mut Element, precision: Option<usize>) {
    fn visit(element: &mut Element, precision: Option<usize>) {
        let mut new_children = Vec::new();
        for child in std::mem::take(&mut element.children) {
            match child {
                XMLNode::Element(mut child_el) => {
                    visit(&mut child_el, precision);
                    if let Some(d) = shape_to_path_d(&child_el, precision) {
                        let mut path = Element::new("path");
                        path.attributes = child_el.attributes.clone();
                        for key in ["x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "width", "height", "points"] {
                            attr_remove(&mut path, key);
                        }
                        path.attributes.insert("d".to_string(), d);
                        path.children = child_el.children;
                        new_children.push(XMLNode::Element(path));
                    } else {
                        new_children.push(XMLNode::Element(child_el));
                    }
                }
                other => new_children.push(other),
            }
        }
        element.children = new_children;
    }
    if let Some(d) = shape_to_path_d(root, precision) {
        root.name = "path".to_string();
        for key in ["x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "width", "height", "points"] {
            attr_remove(root, key);
        }
        root.attributes.insert("d".to_string(), d);
    }
    visit(root, precision);
}

fn cleanup_attrs(root: &mut Element) {
    walk_mut(root, &mut |element| {
        for value in element.attributes.values_mut() {
            *value = value.split_whitespace().collect::<Vec<_>>().join(" ");
        }
    });
}

fn parse_style(style: Option<&str>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(style) = style {
        for part in style.split(';') {
            if let Some((key, value)) = part.split_once(':') {
                out.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }
    out
}

fn style_text(style: &HashMap<String, String>) -> String {
    let mut keys: Vec<_> = style.keys().collect();
    keys.sort();
    keys.into_iter()
        .filter_map(|key| style.get(key).map(|value| format!("{}:{}", key, value)))
        .collect::<Vec<_>>()
        .join(";")
}

fn minify_styles(root: &mut Element) {
    walk_mut(root, &mut |element| {
        if let Some(style) = element.attributes.get("style").cloned() {
            let parsed = parse_style(Some(&style));
            if parsed.is_empty() {
                element.attributes.remove("style");
            } else {
                element.attributes.insert("style".to_string(), style_text(&parsed));
            }
        }
    });
}

fn parse_css_rules(css: &str) -> Vec<(String, HashMap<String, String>)> {
    let mut cleaned = String::new();
    let mut i = 0usize;
    while i < css.len() {
        if css[i..].starts_with("/*") {
            if let Some(end) = css[i + 2..].find("*/") {
                i += end + 4;
                continue;
            }
        }
        cleaned.push(css.as_bytes()[i] as char);
        i += 1;
    }
    let mut rules = Vec::new();
    let mut rest = cleaned.as_str();
    while let Some(open) = rest.find('{') {
        let selector_text = &rest[..open];
        let Some(close) = rest[open + 1..].find('}') else {
            break;
        };
        let body = &rest[open + 1..open + 1 + close];
        let declarations = parse_style(Some(body));
        if !declarations.is_empty() {
            for selector in selector_text.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if !selector.starts_with('@') {
                    rules.push((selector.to_string(), declarations.clone()));
                }
            }
        }
        rest = &rest[open + close + 2..];
    }
    rules
}

fn selector_matches(element: &Element, selector: &str) -> bool {
    if selector.is_empty() || [" ", ">", "+", "~", "[", ":"].iter().any(|t| selector.contains(t)) {
        return false;
    }
    let name = local_name(&element.name);
    let classes: HashSet<_> = attr_value(element, "class")
        .unwrap_or("")
        .split_whitespace()
        .collect();
    let ident = attr_value(element, "id");
    if selector == "*" {
        true
    } else if let Some(class) = selector.strip_prefix('.') {
        classes.contains(class)
    } else if let Some(id) = selector.strip_prefix('#') {
        ident == Some(id)
    } else if let Some((tag, cls)) = selector.split_once('.') {
        tag == name && classes.contains(cls)
    } else if let Some((tag, id)) = selector.split_once('#') {
        tag == name && ident == Some(id)
    } else {
        selector == name
    }
}

fn element_text(element: &Element) -> String {
    let mut out = String::new();
    for child in &element.children {
        match child {
            XMLNode::Text(t) | XMLNode::CData(t) => out.push_str(t),
            XMLNode::Element(e) => out.push_str(&element_text(e)),
            _ => {}
        }
    }
    out
}

fn inline_style_elements(root: &mut Element, remove_style_elements: bool) {
    let mut rules = Vec::new();
    let mut elements = Vec::new();
    walk_ref(root, &mut elements);
    for element in elements {
        if local_name(&element.name) == "style" {
            rules.extend(parse_css_rules(&element_text(element)));
        }
    }
    walk_mut(root, &mut |element| {
        if local_name(&element.name) == "style" {
            return;
        }
        let mut applied = HashMap::new();
        for (selector, declarations) in &rules {
            if selector_matches(element, selector) {
                applied.extend(declarations.clone());
            }
        }
        if !applied.is_empty() {
            let original = parse_style(attr_value(element, "style"));
            applied.extend(original);
            element
                .attributes
                .insert("style".to_string(), style_text(&applied));
        }
    });
    if remove_style_elements {
        remove_children_by(root, &|el| local_name(&el.name) == "style");
    }
}

fn convert_style_to_attrs(root: &mut Element) {
    walk_mut(root, &mut |element| {
        let style = parse_style(attr_value(element, "style"));
        if style.is_empty() {
            return;
        }
        for (key, value) in style {
            element.attributes.entry(key).or_insert(value);
        }
        element.attributes.remove("style");
    });
}

fn cleanup_numeric_values(root: &mut Element, precision: Option<usize>) {
    let decimals = precision.unwrap_or(4);
    let numeric_names: HashSet<&str> = [
        "x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "width", "height",
        "stroke-width", "opacity", "fill-opacity", "stroke-opacity",
    ]
    .into_iter()
    .collect();
    walk_mut(root, &mut |element| {
        for (key, value) in element.attributes.iter_mut() {
            if local_name(key) == "d" {
                continue;
            }
            if numeric_names.contains(local_name(key)) || value.chars().all(|c| c.is_ascii_digit() || "+-.eE, ".contains(c)) {
                *value = reformat_number_list(value, decimals);
            }
        }
    });
}

fn reformat_number_list(value: &str, decimals: usize) -> String {
    let mut out = String::new();
    let mut last = 0usize;
    let chars: Vec<char> = value.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        let starts_num = chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '+' || chars[i] == '-';
        if !starts_num {
            i += 1;
            continue;
        }
        let start = i;
        if chars[i] == '+' || chars[i] == '-' {
            i += 1;
        }
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i < chars.len() && chars[i] == '.' {
            i += 1;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
            i += 1;
            if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                i += 1;
            }
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
        }
        let token: String = chars[start..i].iter().collect();
        if let Ok(num) = token.parse::<f64>() {
            out.push_str(&chars[last..start].iter().collect::<String>());
            out.push_str(&fmt_number(num, decimals, true));
            last = i;
        }
    }
    out.push_str(&chars[last..].iter().collect::<String>());
    out
}

fn min_color(value: &str) -> String {
    match value {
        "white" => "#fff".to_string(),
        "black" => "#000".to_string(),
        "none" => "none".to_string(),
        "currentColor" => "currentColor".to_string(),
        _ => {
            let trimmed = value.trim();
            if trimmed.len() == 7 && trimmed.starts_with('#') && trimmed[1..].chars().all(|c| c.is_ascii_hexdigit()) {
                let hex = trimmed[1..].to_ascii_lowercase();
                let bytes = hex.as_bytes();
                if bytes[0] == bytes[1] && bytes[2] == bytes[3] && bytes[4] == bytes[5] {
                    format!("#{}{}{}", bytes[0] as char, bytes[2] as char, bytes[4] as char)
                } else {
                    format!("#{}", hex)
                }
            } else if trimmed.to_ascii_lowercase().starts_with("rgb(") && trimmed.ends_with(')') {
                let nums = parse_float_list(trimmed);
                if nums.len() >= 3 {
                    min_color(&format!(
                        "#{:02x}{:02x}{:02x}",
                        nums[0].round().clamp(0.0, 255.0) as u8,
                        nums[1].round().clamp(0.0, 255.0) as u8,
                        nums[2].round().clamp(0.0, 255.0) as u8
                    ))
                } else {
                    value.to_string()
                }
            } else {
                value.to_string()
            }
        }
    }
}

fn convert_colors(root: &mut Element) {
    walk_mut(root, &mut |element| {
        for value in element.attributes.values_mut() {
            *value = min_color(value);
        }
        let mut style = parse_style(attr_value(element, "style"));
        if !style.is_empty() {
            for value in style.values_mut() {
                *value = min_color(value);
            }
            element.attributes.insert("style".to_string(), style_text(&style));
        }
    });
}

fn remove_editor_attrs(root: &mut Element) {
    remove_children_by(root, &|el| {
        ["inkscape", "sodipodi", "sketch", "figma"]
            .iter()
            .any(|ns| el.name.contains(ns))
    });
    walk_mut(root, &mut |element| {
        let keys: Vec<_> = element
            .attributes
            .keys()
            .filter(|k| {
                k.starts_with("data-")
                    || ["inkscape", "sodipodi", "sketch", "figma"]
                        .iter()
                        .any(|ns| k.contains(ns))
            })
            .cloned()
            .collect();
        for key in keys {
            element.attributes.remove(&key);
        }
    });
}

fn remove_empty_attrs(root: &mut Element) {
    walk_mut(root, &mut |element| {
        let keys: Vec<_> = element
            .attributes
            .iter()
            .filter(|(_, v)| v.is_empty())
            .map(|(k, _)| k.clone())
            .collect();
        for key in keys {
            element.attributes.remove(&key);
        }
    });
}

fn remove_event_attrs(root: &mut Element) {
    walk_mut(root, &mut |element| {
        let keys: Vec<_> = element
            .attributes
            .keys()
            .filter(|key| local_name(key).to_ascii_lowercase().starts_with("on"))
            .cloned()
            .collect();
        for key in keys {
            element.attributes.remove(&key);
        }
    });
}

fn is_unsafe_url_text(value: &str, remove_external: bool, allow_data_images: bool) -> bool {
    let text = value.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
    text.contains("javascript:")
        || text.contains("vbscript:")
        || text.contains("expression(")
        || (text.starts_with("data:") && !(allow_data_images && text.starts_with("data:image/")))
        || (remove_external
            && (text.starts_with("http:")
                || text.starts_with("https:")
                || text.starts_with("//")
                || text.contains("url(http:")
                || text.contains("url(https:")
                || text.contains("url(//")))
}

fn remove_unsafe_links(root: &mut Element, remove_external: bool, allow_data_images: bool) {
    remove_children_by(root, &|el| {
        local_name(&el.name) == "style"
            && is_unsafe_url_text(&element_text(el), remove_external, allow_data_images)
    });
    walk_mut(root, &mut |element| {
        let keys: Vec<_> = element
            .attributes
            .iter()
            .filter(|(key, value)| {
                let lname = local_name(key);
                (["href", "src", "style"].contains(&lname))
                    && is_unsafe_url_text(value, remove_external, allow_data_images)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in keys {
            element.attributes.remove(&key);
        }
    });
}

fn remove_empty_containers(root: &mut Element) {
    let containers: HashSet<&str> = ["g", "defs", "clipPath", "mask", "pattern", "symbol", "marker"]
        .into_iter()
        .collect();
    loop {
        let mut changed = false;
        fn visit(element: &mut Element, containers: &HashSet<&str>, changed: &mut bool) {
            element.children.retain(|node| {
                if let XMLNode::Element(el) = node {
                    let empty = containers.contains(local_name(&el.name))
                        && el.children.iter().all(|child| match child {
                            XMLNode::Text(t) => t.trim().is_empty(),
                            XMLNode::Element(_) => false,
                            _ => true,
                        });
                    if empty {
                        *changed = true;
                    }
                    !empty
                } else {
                    true
                }
            });
            for child in &mut element.children {
                if let XMLNode::Element(el) = child {
                    visit(el, containers, changed);
                }
            }
        }
        visit(root, &containers, &mut changed);
        if !changed {
            break;
        }
    }
}

fn collapse_groups(root: &mut Element) {
    loop {
        let mut changed = false;
        fn visit(element: &mut Element, changed: &mut bool) {
            let mut new_children = Vec::new();
            for child in std::mem::take(&mut element.children) {
                match child {
                    XMLNode::Element(mut el) => {
                        visit(&mut el, changed);
                        if local_name(&el.name) == "g" && el.attributes.is_empty() {
                            new_children.extend(el.children);
                            *changed = true;
                        } else {
                            new_children.push(XMLNode::Element(el));
                        }
                    }
                    other => new_children.push(other),
                }
            }
            element.children = new_children;
        }
        visit(root, &mut changed);
        if !changed {
            break;
        }
    }
}

fn remove_hidden(root: &mut Element) {
    remove_children_by(root, &|el| {
        let style = parse_style(attr_value(el, "style"));
        attr_value(el, "display") == Some("none")
            || attr_value(el, "visibility") == Some("hidden")
            || style.get("display").is_some_and(|v| v == "none")
            || style.get("visibility").is_some_and(|v| v == "hidden")
    });
}

fn remove_empty_text(root: &mut Element) {
    remove_children_by(root, &|el| {
        ["text", "tspan"].contains(&local_name(&el.name)) && element_text(el).trim().is_empty() && child_element_count(el) == 0
    });
}

fn child_element_count(element: &Element) -> usize {
    element
        .children
        .iter()
        .filter(|n| matches!(n, XMLNode::Element(_)))
        .count()
}

fn remove_empty_defs(root: &mut Element) {
    loop {
        let mut changed = false;
        fn visit(element: &mut Element, changed: &mut bool) {
            for child in &mut element.children {
                if let XMLNode::Element(el) = child {
                    if local_name(&el.name) == "defs" {
                        el.children.retain(|node| {
                            if let XMLNode::Element(child_el) = node {
                                let keep = attr_value(child_el, "id").is_some();
                                if !keep {
                                    *changed = true;
                                }
                                keep
                            } else {
                                true
                            }
                        });
                    }
                    visit(el, changed);
                }
            }
            element.children.retain(|node| {
                if let XMLNode::Element(el) = node {
                    let remove = local_name(&el.name) == "defs" && child_element_count(el) == 0;
                    if remove {
                        *changed = true;
                    }
                    !remove
                } else {
                    true
                }
            });
        }
        visit(root, &mut changed);
        if !changed {
            break;
        }
    }
}

fn remove_defaults(root: &mut Element) {
    walk_mut(root, &mut |element| {
        if attr_value(element, "version") == Some("1.1") {
            attr_remove(element, "version");
        }
        if attr_value(element, "type") == Some("text/css") {
            attr_remove(element, "type");
        }
    });
}

fn remove_non_inheritable_group_attrs(root: &mut Element) {
    walk_mut(root, &mut |element| {
        if local_name(&element.name) == "g" {
            for key in ["x", "y", "width", "height"] {
                attr_remove(element, key);
            }
        }
    });
}

fn remove_useless_stroke_fill(root: &mut Element) {
    walk_mut(root, &mut |element| {
        if attr_value(element, "stroke") == Some("none") {
            for key in ["stroke-width", "stroke-linecap", "stroke-linejoin", "stroke-opacity"] {
                attr_remove(element, key);
            }
        }
        if attr_value(element, "fill") == Some("none") && attr_value(element, "stroke").is_none() {
            attr_remove(element, "fill-opacity");
        }
    });
}

fn convert_path_data(root: &mut Element, precision: Option<usize>) {
    let decimals = precision.unwrap_or(4);
    walk_mut(root, &mut |element| {
        if local_name(&element.name) == "path" {
            if let Some(d) = attr_value(element, "d").map(str::to_string) {
                if let Ok(mut path) = PathDataCore::parse(&d) {
                    if path.optimize(Some("safe")).is_ok() {
                        element.attributes.insert("d".to_string(), path.to_string(decimals, true));
                    }
                }
            }
        }
    });
}

fn transform_subtree_is_flattenable(element: &Element) -> bool {
    let geometry: HashSet<&str> = ["path", "rect", "line", "polyline", "polygon", "circle", "ellipse"].into_iter().collect();
    let containers: HashSet<&str> = ["svg", "g", "defs", "clipPath", "mask", "pattern", "symbol", "marker"].into_iter().collect();
    let mut items = Vec::new();
    walk_ref(element, &mut items);
    items.into_iter().all(|el| {
        let name = local_name(&el.name);
        geometry.contains(name) || containers.contains(name)
    })
}

fn apply_transform_to_element(element: &mut Element, matrix: Matrix, decimals: usize, precision: Option<usize>) {
    if matrix == IDENTITY {
        return;
    }
    let name = local_name(&element.name).to_string();
    if name == "path" {
        if let Some(d) = attr_value(element, "d").map(str::to_string) {
            if let Ok(mut path) = PathDataCore::parse(&d) {
                if path.transform(matrix).is_ok() {
                    element.attributes.insert("d".to_string(), path.to_string(decimals, true));
                }
            }
        }
    } else if ["rect", "line", "polyline", "polygon", "circle", "ellipse"].contains(&name.as_str()) {
        if let Some(d) = shape_to_path_d(element, precision) {
            if let Ok(mut path) = PathDataCore::parse(&d) {
                if path.transform(matrix).is_ok() {
                    element.name = "path".to_string();
                    for key in ["x", "y", "x1", "y1", "x2", "y2", "cx", "cy", "r", "rx", "ry", "width", "height", "points", "transform"] {
                        attr_remove(element, key);
                    }
                    element.attributes.insert("d".to_string(), path.to_string(decimals, true));
                }
            }
        }
    }
}

fn convert_transforms(root: &mut Element, precision: Option<usize>) {
    let decimals = precision.unwrap_or(4);
    fn visit(element: &mut Element, inherited: Matrix, decimals: usize, precision: Option<usize>) {
        let local_matrix = attr_value(element, "transform")
            .and_then(|t| parse_transform(t).ok())
            .unwrap_or(IDENTITY);
        let matrix = matrix_multiply(inherited, local_matrix);
        let name = local_name(&element.name).to_string();
        apply_transform_to_element(element, matrix, decimals, precision);
        let container_flattenable = ["svg", "g", "defs", "clipPath", "mask", "pattern", "symbol", "marker"].contains(&name.as_str())
            && transform_subtree_is_flattenable(element);
        if ["path", "rect", "line", "polyline", "polygon", "circle", "ellipse"].contains(&name.as_str()) || container_flattenable {
            attr_remove(element, "transform");
        }
        let child_inherited = if container_flattenable || !["svg", "g", "defs", "clipPath", "mask", "pattern", "symbol", "marker"].contains(&name.as_str()) {
            matrix
        } else {
            IDENTITY
        };
        for child in &mut element.children {
            if let XMLNode::Element(child_el) = child {
                visit(child_el, child_inherited, decimals, precision);
            }
        }
    }
    visit(root, IDENTITY, decimals, precision);
}

fn merge_paths(root: &mut Element) {
    fn attrs_without_d(element: &Element) -> Vec<(String, String)> {
        let mut attrs: Vec<_> = element
            .attributes
            .iter()
            .filter(|(k, _)| local_name(k) != "d")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        attrs.sort();
        attrs
    }
    fn visit(element: &mut Element) {
        let mut merged: Vec<XMLNode> = Vec::new();
        for child in std::mem::take(&mut element.children) {
            match child {
                XMLNode::Element(mut el) => {
                    visit(&mut el);
                    if local_name(&el.name) == "path" && attr_value(&el, "d").is_some() {
                        if let Some(XMLNode::Element(prev)) = merged.last_mut() {
                            if local_name(&prev.name) == "path" && attr_value(prev, "d").is_some() && attrs_without_d(prev) == attrs_without_d(&el) {
                                let d = format!("{} {}", attr_value(prev, "d").unwrap_or(""), attr_value(&el, "d").unwrap_or("")).trim().to_string();
                                prev.attributes.insert("d".to_string(), d);
                                continue;
                            }
                        }
                    }
                    merged.push(XMLNode::Element(el));
                }
                other => merged.push(other),
            }
        }
        element.children = merged;
    }
    visit(root);
}

fn sort_attrs(root: &mut Element) {
    walk_mut(root, &mut |element| {
        let mut items: Vec<_> = element.attributes.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        items.sort_by_key(|(k, _)| (if local_name(k) == "d" { 0 } else { 1 }, local_name(k).to_string()));
        element.attributes.clear();
        for (k, v) in items {
            element.attributes.insert(k, v);
        }
    });
}

fn sort_defs_children(root: &mut Element) {
    walk_mut(root, &mut |element| {
        if local_name(&element.name) == "defs" {
            element.children.sort_by(|a, b| match (a, b) {
                (XMLNode::Element(a), XMLNode::Element(b)) => {
                    (local_name(&a.name), attr_value(a, "id").unwrap_or("")).cmp(&(local_name(&b.name), attr_value(b, "id").unwrap_or("")))
                }
                _ => Ordering::Equal,
            });
        }
    });
}

fn prefix_ids(root: &mut Element, prefix: &str) {
    let mut mapping = HashMap::new();
    walk_mut(root, &mut |element| {
        if let Some(id) = attr_value(element, "id").map(str::to_string) {
            let new = format!("{}{}", prefix, id);
            mapping.insert(id, new.clone());
            element.attributes.insert("id".to_string(), new);
        }
    });
    if mapping.is_empty() {
        return;
    }
    walk_mut(root, &mut |element| {
        for value in element.attributes.values_mut() {
            for (old, new) in &mapping {
                *value = value.replace(&format!("#{}", old), &format!("#{}", new));
                *value = value.replace(&format!("url({})", old), &format!("url({})", new));
            }
        }
    });
}

fn attr_matches(name: &str, pattern: &str) -> bool {
    pattern == "*"
        || pattern == name
        || (pattern.ends_with('*') && name.starts_with(&pattern[..pattern.len() - 1]))
        || (pattern.starts_with('/') && pattern.ends_with('/') && name.contains(&pattern[1..pattern.len() - 1]))
}

fn remove_attrs_plugin(root: &mut Element, params: &Value) {
    let attrs = params
        .get("attrs")
        .or_else(|| params.get("attributes"))
        .or_else(|| params.get("name"));
    let patterns: Vec<String> = match attrs {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(items)) => items.iter().filter_map(|v| v.as_str().map(str::to_string)).collect(),
        _ => return,
    };
    walk_mut(root, &mut |element| {
        let keys: Vec<_> = element
            .attributes
            .keys()
            .filter(|key| patterns.iter().any(|pattern| attr_matches(local_name(key), pattern)))
            .cloned()
            .collect();
        for key in keys {
            element.attributes.remove(&key);
        }
    });
}

fn remove_elements_by_attr(root: &mut Element, params: &Value) {
    let Some(map) = params.as_object() else {
        return;
    };
    remove_children_by(root, &|el| {
        map.iter().all(|(key, value)| {
            attr_value(el, key).is_some_and(|actual| actual == value.as_str().unwrap_or(&value.to_string()))
        })
    });
}

fn remove_xlink(root: &mut Element) {
    walk_mut(root, &mut |element| {
        let keys: Vec<_> = element
            .attributes
            .keys()
            .filter(|key| key.contains("xlink"))
            .cloned()
            .collect();
        for key in keys {
            if local_name(&key) == "href" && attr_value(element, "href").is_none() {
                if let Some(value) = element.attributes.get(&key).cloned() {
                    element.attributes.insert("href".to_string(), value);
                }
            }
            element.attributes.remove(&key);
        }
    });
}

fn apply_plugin(root: &mut Element, plugin: &PluginSpecCore, options: &OptimizeOptionsCore) -> Result<()> {
    let name = plugin.name.as_str();
    let builtins = builtin_plugins();
    if !builtins.iter().any(|p| p == name) && name != "preset-default" {
        return Err(SvgoError(format!("Unknown SVGO plugin: {}", name)));
    }
    match name {
        "removeDoctype" | "removeXMLProcInst" | "removeComments" | "removeUnusedNS" | "reusePaths" | "preset-default" => {}
        "removeMetadata" => remove_children_by(root, &|el| local_name(&el.name) == "metadata"),
        "removeDesc" => remove_children_by(root, &|el| local_name(&el.name) == "desc"),
        "removeTitle" => remove_children_by(root, &|el| local_name(&el.name) == "title"),
        "removeScripts" | "removeScriptElement" => remove_children_by(root, &|el| local_name(&el.name) == "script"),
        "removeStyleElement" => remove_children_by(root, &|el| local_name(&el.name) == "style"),
        "removeRasterImages" => remove_children_by(root, &|el| local_name(&el.name) == "image"),
        "removeEditorsNSData" => remove_editor_attrs(root),
        "cleanupAttrs" => cleanup_attrs(root),
        "mergeStyles" | "minifyStyles" => minify_styles(root),
        "inlineStyles" => {
            inline_style_elements(root, plugin.params.get("removeStyleElement").and_then(Value::as_bool).unwrap_or(false));
            minify_styles(root);
        }
        "cleanupIds" | "cleanupIDs" => cleanup_ids(root),
        "removeUselessDefs" => remove_empty_defs(root),
        "cleanupNumericValues" | "cleanupListOfValues" => cleanup_numeric_values(root, options.float_precision),
        "convertColors" => convert_colors(root),
        "removeUnknownsAndDefaults" => remove_defaults(root),
        "removeNonInheritableGroupAttrs" => remove_non_inheritable_group_attrs(root),
        "removeUselessStrokeAndFill" => remove_useless_stroke_fill(root),
        "cleanupEnableBackground" => {
            walk_mut(root, &mut |el| attr_remove(el, "enable-background"));
        }
        "removeHiddenElems" => remove_hidden(root),
        "removeEmptyText" => remove_empty_text(root),
        "convertShapeToPath" | "convertEllipseToCircle" => convert_shapes_to_paths(root, options.float_precision),
        "convertTransform" => convert_transforms(root, options.float_precision),
        "convertPathData" => convert_path_data(root, options.float_precision),
        "removeEmptyAttrs" => remove_empty_attrs(root),
        "removeEmptyContainers" => remove_empty_containers(root),
        "collapseGroups" => collapse_groups(root),
        "moveElemsAttrsToGroup" | "moveGroupAttrsToElems" | "convertOneStopGradients" => {}
        "mergePaths" => merge_paths(root),
        "sortAttrs" => sort_attrs(root),
        "sortDefsChildren" => sort_defs_children(root),
        "addAttributesToSVGElement" => {
            if let Some(map) = plugin.params.as_object() {
                for (key, value) in map {
                    root.attributes.insert(key.clone(), value.as_str().unwrap_or(&value.to_string()).to_string());
                }
            }
        }
        "addClassesToSVGElement" => {
            let classes = plugin.params.get("classNames").or_else(|| plugin.params.get("classes")).or_else(|| plugin.params.get("class"));
            let mut list = Vec::new();
            match classes {
                Some(Value::String(s)) => list.push(s.clone()),
                Some(Value::Array(items)) => list.extend(items.iter().filter_map(|v| v.as_str().map(str::to_string))),
                _ => {}
            }
            if !list.is_empty() {
                let existing = attr_value(root, "class").unwrap_or("");
                let value = [existing.to_string(), list.join(" ")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
                root.attributes.insert("class".to_string(), value);
            }
        }
        "convertStyleToAttrs" => convert_style_to_attrs(root),
        "prefixIds" => prefix_ids(root, plugin.params.get("prefix").and_then(Value::as_str).unwrap_or("prefix")),
        "removeAttrs" | "removeAttributesBySelector" => remove_attrs_plugin(root, &plugin.params),
        "removeDimensions" => {
            attr_remove(root, "width");
            attr_remove(root, "height");
        }
        "removeElementsByAttr" => remove_elements_by_attr(root, &plugin.params),
        "removeOffCanvasPaths" => {}
        "removeEventAttributes" => remove_event_attrs(root),
        "removeUnsafeLinks" => remove_unsafe_links(
            root,
            plugin.params.get("removeExternal").and_then(Value::as_bool).unwrap_or(false),
            plugin.params.get("allowDataImages").and_then(Value::as_bool).unwrap_or(true),
        ),
        "removeViewBox" => attr_remove(root, "viewBox"),
        "removeXlink" => remove_xlink(root),
        "removeXMLNS" => {}
        _ => {}
    }
    Ok(())
}

fn cleanup_ids(root: &mut Element) {
    let serialized = serialize_element(root, false, 2).unwrap_or_default();
    walk_mut(root, &mut |element| {
        if let Some(id) = attr_value(element, "id").map(str::to_string) {
            let refs = serialized.matches(&format!("#{}", id)).count()
                + serialized.matches(&format!("url(&quot;#{}&quot;)", id)).count()
                + serialized.matches(&format!("url('#{}')", id)).count();
            if refs == 0 {
                attr_remove(element, "id");
            }
        }
    });
}

fn optimize_svg_core(svg_text: &str, options: OptimizeOptionsCore) -> Result<String> {
    let passes = if options.multipass { 10 } else { 1 };
    let mut result = svg_text.to_string();
    for pass_index in 0..passes {
        let previous = result.clone();
        let current = optimize_once(&result, &options)?;
        if options.multipass && pass_index > 0 && current.len() >= previous.len() {
            break;
        }
        result = current;
        if !options.multipass || result.len() >= previous.len() {
            break;
        }
    }
    if let Some(mode) = &options.datauri {
        result = to_data_uri(&result, mode)?;
    }
    if options.eol.as_deref() == Some("crlf") {
        result = result.replace('\n', "\r\n");
    } else if options.eol.as_deref() == Some("lf") {
        result = result.replace("\r\n", "\n");
    }
    if options.final_newline && !result.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

fn optimize_once(svg_text: &str, options: &OptimizeOptionsCore) -> Result<String> {
    let plugins = effective_plugins(options)?;
    let names: HashSet<_> = plugins.iter().map(|p| p.name.as_str()).collect();
    let mut text = svg_text.to_string();
    if names.contains("removeDoctype") {
        text = remove_doctype(text);
    }
    if names.contains("removeXMLProcInst") {
        text = remove_xml_proc(text);
    }
    if names.contains("removeComments") {
        text = remove_comments(text);
    }
    let mut root = parse_svg_element(&text)?;
    if local_name(&root.name) == "svg" && attr_value(&root, "xmlns").is_none() {
        root.attributes.insert("xmlns".to_string(), SVG_NS.to_string());
    }
    for plugin in &plugins {
        apply_plugin(&mut root, plugin, options)?;
    }
    let mut out = serialize_element(&root, options.pretty, options.indent)?;
    if names.contains("removeXMLNS") {
        out = remove_xmlns_attrs(&out);
    }
    Ok(out)
}

fn remove_xmlns_attrs(text: &str) -> String {
    let mut out = String::new();
    for part in text.split_whitespace() {
        if part.starts_with("xmlns") {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

fn to_data_uri(svg_text: &str, mode: &str) -> Result<String> {
    match mode {
        "base64" => Ok(format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(svg_text.as_bytes())
        )),
        "enc" => Ok(format!(
            "data:image/svg+xml,{}",
            utf8_percent_encode(svg_text, DATA_URI_SAFE)
        )),
        "unenc" => Ok(format!("data:image/svg+xml,{}", svg_text)),
        _ => Err("--svgo-datauri must be base64, enc, or unenc".into()),
    }
}

#[pyfunction]
fn builtin_plugins_json() -> String {
    serde_json::to_string(&builtin_plugins()).unwrap()
}

#[pyfunction]
fn optimize_svg(svg_text: &str, options_json: Option<&str>) -> PyResult<String> {
    optimize_svg_core(svg_text, parse_opt_options_json(options_json).map_err(py_err)?).map_err(py_err)
}

fn read_svg_text(input: &str) -> (String, Option<String>, Option<String>) {
    if input.trim_start().starts_with('<') {
        return (input.to_string(), None, None);
    }
    let path = Path::new(input);
    if path.exists() {
        match fs::read_to_string(path) {
            Ok(text) => (text, Some(input.to_string()), None),
            Err(err) => (String::new(), Some(input.to_string()), Some(err.to_string())),
        }
    } else {
        (input.to_string(), None, None)
    }
}

fn validate_svg_value(svg_input: &str, strict: bool) -> Value {
    let (text, source, read_error) = read_svg_text(svg_input);
    if let Some(err) = read_error {
        return json!({"valid": false, "issues": [{"level": "error", "reason": err}], "error": err});
    }
    let root = match parse_svg_element(&text) {
        Ok(root) => root,
        Err(err) => {
            let reason = format!("XML parse error: {}", err.0.trim_start_matches("Could not parse SVG: "));
            return json!({"valid": false, "issues": [{"level": "error", "reason": reason}], "error": reason});
        }
    };
    let mut issues = Vec::new();
    if local_name(&root.name) != "svg" {
        issues.push(json!({"level": "error", "reason": "Root element is not <svg>"}));
    }
    if attr_value(&root, "viewBox").is_none() && !(attr_value(&root, "width").is_some() && attr_value(&root, "height").is_some()) {
        issues.push(json!({"level": "warning", "reason": "SVG has neither viewBox nor width/height dimensions"}));
    }
    let known: HashSet<&str> = [
        "svg", "g", "defs", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "text", "tspan", "image",
        "style", "script", "metadata", "desc", "title", "linearGradient", "radialGradient", "stop", "clipPath", "mask",
        "pattern", "symbol", "use", "marker", "a",
    ]
    .into_iter()
    .collect();
    let mut elements = Vec::new();
    walk_ref(&root, &mut elements);
    for element in elements {
        let name = local_name(&element.name);
        if name == "script" {
            issues.push(json!({"level": "error", "reason": "SVG contains a <script> element"}));
        }
        if !known.contains(name) {
            issues.push(json!({"level": "warning", "reason": format!("Unknown or uncommon SVG element: {}", name)}));
        }
        for (key, value) in &element.attributes {
            let attr = local_name(key);
            if attr.to_ascii_lowercase().starts_with("on") {
                issues.push(json!({"level": "error", "reason": format!("Event handler attribute on <{}>: {}", name, attr)}));
            }
            if attr.contains("href") && value.trim().to_ascii_lowercase().starts_with("javascript:") {
                issues.push(json!({"level": "error", "reason": format!("Potentially unsafe href on <{}>", name)}));
            }
            if attr == "style" && is_unsafe_url_text(value, false, true) {
                issues.push(json!({"level": "error", "reason": format!("Potentially unsafe style on <{}>", name)}));
            }
            if ["inkscape", "sodipodi", "sketch", "figma"].iter().any(|ns| key.contains(ns)) {
                issues.push(json!({"level": "warning", "reason": format!("Editor-specific attribute on <{}>: {}", name, attr)}));
            }
        }
    }
    let has_error = issues.iter().any(|i| i.get("level").and_then(Value::as_str) == Some("error"));
    let valid = if strict { issues.is_empty() } else { !has_error };
    json!({"valid": valid, "issues": issues, "error": Value::Null, "source": source})
}

#[pyfunction]
fn validate_svg_json(svg_input: &str, strict: bool) -> String {
    validate_svg_value(svg_input, strict).to_string()
}

fn collect_fonts(root: &Element) -> Vec<String> {
    let mut fonts = HashSet::new();
    let mut elements = Vec::new();
    walk_ref(root, &mut elements);
    for element in elements {
        if let Some(value) = attr_value(element, "font-family") {
            for font in split_font_families(value) {
                fonts.insert(font);
            }
        }
        let style = parse_style(attr_value(element, "style"));
        if let Some(value) = style.get("font-family") {
            for font in split_font_families(value) {
                fonts.insert(font);
            }
        }
        if local_name(&element.name) == "style" {
            let text = element_text(element);
            for part in text.split(';') {
                if let Some((key, value)) = part.split_once(':') {
                    if key.trim().eq_ignore_ascii_case("font-family") {
                        for font in split_font_families(value) {
                            fonts.insert(font);
                        }
                    }
                }
            }
        }
    }
    let mut out: Vec<_> = fonts.into_iter().collect();
    out.sort();
    out
}

fn split_font_families(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|part| part.trim().trim_matches(['"', '\'']).to_string())
        .filter(|part| !part.is_empty())
        .collect()
}

fn get_svg_info_value(svg_input: &str) -> Value {
    let (text, source, read_error) = read_svg_text(svg_input);
    if let Some(err) = read_error {
        return json!({"error": err});
    }
    let root = match parse_svg_element(&text) {
        Ok(root) => root,
        Err(err) => return json!({"error": format!("XML parse error: {}", err.0.trim_start_matches("Could not parse SVG: "))}),
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    let mut elements = Vec::new();
    walk_ref(&root, &mut elements);
    for element in &elements {
        *counts.entry(local_name(&element.name).to_string()).or_default() += 1;
    }
    let shape_names = ["rect", "circle", "ellipse", "line", "polyline", "polygon"];
    let shapes: usize = shape_names.iter().map(|name| counts.get(*name).copied().unwrap_or(0)).sum();
    json!({
        "source": source,
        "width": attr_value(&root, "width"),
        "height": attr_value(&root, "height"),
        "viewBox": attr_value(&root, "viewBox"),
        "elements": elements.len(),
        "element_counts": counts,
        "paths": counts.get("path").copied().unwrap_or(0),
        "shapes": shapes,
        "text": counts.get("text").copied().unwrap_or(0) + counts.get("tspan").copied().unwrap_or(0),
        "images": counts.get("image").copied().unwrap_or(0),
        "fonts": collect_fonts(&root),
        "bytes": text.len(),
    })
}

#[pyfunction]
fn get_svg_info_json(svg_input: &str) -> String {
    get_svg_info_value(svg_input).to_string()
}

fn iter_svg_paths_value(svg_text: &str, decimals: Option<usize>) -> Result<Value> {
    let root = parse_svg_element(svg_text)?;
    let mut warnings = Vec::new();
    let mut items = Vec::new();
    fn walk(element: &Element, inherited: Matrix, decimals: Option<usize>, warnings: &mut Vec<String>, items: &mut Vec<Value>) {
        let mut matrix = inherited;
        if let Some(transform) = attr_value(element, "transform") {
            match parse_transform(transform) {
                Ok(local) => matrix = matrix_multiply(inherited, local),
                Err(err) => warnings.push(format!("Could not parse transform on <{}>: {}", local_name(&element.name), err)),
            }
        }
        let name = local_name(&element.name);
        let mut d = if name == "path" {
            attr_value(element, "d").map(str::to_string)
        } else {
            shape_to_path_d(element, decimals)
        };
        if let Some(path_d) = d.take() {
            let transformed = if matrix != IDENTITY {
                let mut path = match PathDataCore::parse(&path_d) {
                    Ok(path) => path,
                    Err(err) => {
                        warnings.push(format!("Could not parse <{}> geometry: {}", name, err));
                        return;
                    }
                };
                match path.transform(matrix) {
                    Ok(_) => path.to_string(decimals.unwrap_or(4), true),
                    Err(err) => {
                        warnings.push(format!("Could not transform <{}> geometry: {}", name, err));
                        path_d
                    }
                }
            } else {
                path_d
            };
            items.push(json!({"element": name, "id": attr_value(element, "id").unwrap_or(""), "d": transformed}));
        }
        for child in &element.children {
            if let XMLNode::Element(child_el) = child {
                walk(child_el, matrix, decimals, warnings, items);
            }
        }
    }
    walk(&root, IDENTITY, decimals, &mut warnings, &mut items);
    Ok(json!({"paths": items, "warnings": warnings}))
}

fn svg_metrics_value(svg_input: &str, decimals: Option<usize>, error: f64) -> Value {
    let (text, source, read_error) = read_svg_text(svg_input);
    if let Some(err) = read_error {
        return json!({"error": err, "source": source});
    }
    let iter = match iter_svg_paths_value(&text, decimals) {
        Ok(v) => v,
        Err(err) => return json!({"error": format!("XML parse error: {}", err), "source": source}),
    };
    let warnings = iter.get("warnings").cloned().unwrap_or_else(|| json!([]));
    let raw_paths = iter.get("paths").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut paths = Vec::new();
    let mut total = 0.0;
    let mut overall: Option<Bounds> = None;
    for (index, item) in raw_paths.into_iter().enumerate() {
        let d = item.get("d").and_then(Value::as_str).unwrap_or("");
        if let Ok(metrics) = path_metrics_value(d, decimals, error) {
            let length = metrics.get("length").and_then(Value::as_f64).unwrap_or(0.0);
            total += length;
            if let Some(bbox) = metrics.get("bbox").and_then(Value::as_object) {
                let b = Bounds {
                    x: bbox.get("x").and_then(Value::as_f64).unwrap_or(0.0),
                    y: bbox.get("y").and_then(Value::as_f64).unwrap_or(0.0),
                    x2: bbox.get("x2").and_then(Value::as_f64).unwrap_or(0.0),
                    y2: bbox.get("y2").and_then(Value::as_f64).unwrap_or(0.0),
                };
                if let Some(overall) = &mut overall {
                    overall.include_point(Point::new(b.x, b.y));
                    overall.include_point(Point::new(b.x2, b.y2));
                } else {
                    overall = Some(b);
                }
            }
            let mut entry = metrics;
            if let Some(map) = entry.as_object_mut() {
                map.insert("index".to_string(), json!(index));
                map.insert("element".to_string(), item.get("element").cloned().unwrap_or(Value::Null));
                map.insert("id".to_string(), item.get("id").cloned().unwrap_or(Value::Null));
                map.insert("d".to_string(), json!(d));
            }
            paths.push(entry);
        }
    }
    json!({
        "source": source,
        "length": round_json(total, decimals),
        "bbox": overall.map(|b| b.to_json(decimals)).unwrap_or(Value::Null),
        "paths": paths,
        "path_count": paths.len(),
        "warnings": warnings,
    })
}

#[pyfunction]
fn svg_metrics_json(svg_input: &str, decimals: Option<usize>, error: Option<f64>) -> String {
    svg_metrics_value(svg_input, decimals, error.unwrap_or(0.01)).to_string()
}

fn to_plain_svg_core(svg_text: &str, precision: Option<usize>) -> Result<String> {
    optimize_svg_core(
        svg_text,
        OptimizeOptionsCore {
            preset: "none".to_string(),
            plugins: vec![
                plugin("removeComments"),
                plugin("removeMetadata"),
                plugin("removeEditorsNSData"),
                plugin("cleanupAttrs"),
                plugin("removeEmptyContainers"),
                plugin("sortAttrs"),
            ],
            float_precision: precision,
            ..Default::default()
        },
    )
}

fn plugin(name: &str) -> PluginSpecCore {
    PluginSpecCore {
        name: name.to_string(),
        params: Value::Object(Default::default()),
    }
}

fn plugin_params(name: &str, params: Value) -> PluginSpecCore {
    PluginSpecCore {
        name: name.to_string(),
        params,
    }
}

#[pyfunction]
fn to_plain_svg(svg_text: &str, precision: Option<usize>) -> PyResult<String> {
    to_plain_svg_core(svg_text, precision).map_err(py_err)
}

fn sanitize_svg_core(svg_text: &str, precision: Option<usize>, remove_external_refs: bool, allow_data_images: bool, remove_styles: bool, remove_raster_images: bool) -> Result<String> {
    let mut plugins = vec![
        plugin("removeComments"),
        plugin("removeScripts"),
        plugin("removeScriptElement"),
        plugin_params("removeUnsafeLinks", json!({"removeExternal": remove_external_refs, "allowDataImages": allow_data_images})),
        plugin("removeEventAttributes"),
        plugin("cleanupAttrs"),
        plugin("removeEmptyAttrs"),
        plugin("removeEmptyContainers"),
        plugin("sortAttrs"),
    ];
    if remove_styles {
        plugins.insert(3, plugin("removeStyleElement"));
        plugins.push(plugin_params("removeAttrs", json!({"attrs": "style"})));
    }
    if remove_raster_images {
        plugins.insert(3, plugin("removeRasterImages"));
    }
    optimize_svg_core(
        svg_text,
        OptimizeOptionsCore {
            preset: "none".to_string(),
            plugins,
            float_precision: precision,
            ..Default::default()
        },
    )
}

#[pyfunction]
#[pyo3(signature = (svg_text, precision=None, remove_external_refs=false, allow_data_images=true, remove_styles=false, remove_raster_images=false))]
fn sanitize_svg(
    svg_text: &str,
    precision: Option<usize>,
    remove_external_refs: bool,
    allow_data_images: bool,
    remove_styles: bool,
    remove_raster_images: bool,
) -> PyResult<String> {
    sanitize_svg_core(
        svg_text,
        precision,
        remove_external_refs,
        allow_data_images,
        remove_styles,
        remove_raster_images,
    )
    .map_err(py_err)
}

#[pyfunction]
#[pyo3(signature = (svg_text, precision=None, remove_style_elements=true))]
fn inline_styles_svg(svg_text: &str, precision: Option<usize>, remove_style_elements: bool) -> PyResult<String> {
    optimize_svg_core(
        svg_text,
        OptimizeOptionsCore {
            preset: "none".to_string(),
            plugins: vec![
                plugin_params("inlineStyles", json!({"removeStyleElement": remove_style_elements})),
                plugin("convertStyleToAttrs"),
                plugin("cleanupAttrs"),
                plugin("removeEmptyContainers"),
                plugin("sortAttrs"),
            ],
            float_precision: precision,
            ..Default::default()
        },
    )
    .map_err(py_err)
}

#[pyfunction]
fn convert_shapes_svg(svg_text: &str, precision: Option<usize>) -> PyResult<String> {
    optimize_svg_core(
        svg_text,
        OptimizeOptionsCore {
            preset: "none".to_string(),
            plugins: vec![plugin("convertShapeToPath"), plugin("sortAttrs")],
            float_precision: precision,
            ..Default::default()
        },
    )
    .map_err(py_err)
}

#[pyfunction]
#[pyo3(signature = (svg_text, precision=None, flatten_transforms=true, flatten_groups=true, shapes_to_paths=true, plain=false))]
fn flatten_svg(
    svg_text: &str,
    precision: Option<usize>,
    flatten_transforms: bool,
    flatten_groups: bool,
    shapes_to_paths: bool,
    plain: bool,
) -> PyResult<String> {
    let mut plugins = Vec::new();
    if plain {
        plugins.extend([plugin("removeComments"), plugin("removeMetadata"), plugin("removeEditorsNSData"), plugin("cleanupAttrs")]);
    }
    if shapes_to_paths {
        plugins.push(plugin("convertShapeToPath"));
    }
    if flatten_transforms {
        plugins.push(plugin("convertTransform"));
    }
    if flatten_groups {
        plugins.push(plugin("collapseGroups"));
    }
    plugins.extend([plugin("removeEmptyContainers"), plugin("sortAttrs")]);
    optimize_svg_core(
        svg_text,
        OptimizeOptionsCore {
            preset: "none".to_string(),
            plugins,
            float_precision: precision,
            ..Default::default()
        },
    )
    .map_err(py_err)
}

fn parse_viewbox(value: &str) -> Result<[f64; 4]> {
    let values = parse_float_list(value);
    if values.len() != 4 {
        return Err("viewBox requires four numbers: min-x min-y width height".into());
    }
    Ok([values[0], values[1], values[2], values[3]])
}

fn format_viewbox(values: [f64; 4], precision: Option<usize>) -> String {
    let decimals = precision.unwrap_or(6);
    values
        .iter()
        .map(|v| fmt_number(*v, decimals, false))
        .collect::<Vec<_>>()
        .join(" ")
}

fn set_viewbox_svg_core(svg_text: &str, viewbox: &str, precision: Option<usize>, remove_dimensions: bool) -> Result<String> {
    let mut root = parse_svg_element(svg_text)?;
    if local_name(&root.name) != "svg" {
        return Err("Root element is not <svg>".into());
    }
    let values = parse_viewbox(viewbox)?;
    if values[2] < 0.0 || values[3] < 0.0 {
        return Err("viewBox width and height must be non-negative".into());
    }
    root.attributes.insert("viewBox".to_string(), format_viewbox(values, precision));
    if remove_dimensions {
        attr_remove(&mut root, "width");
        attr_remove(&mut root, "height");
    }
    serialize_element(&root, false, 2)
}

#[pyfunction]
#[pyo3(signature = (svg_text, viewbox, precision=None, remove_dimensions=false))]
fn set_viewbox_svg(svg_text: &str, viewbox: &str, precision: Option<usize>, remove_dimensions: bool) -> PyResult<String> {
    set_viewbox_svg_core(svg_text, viewbox, precision, remove_dimensions).map_err(py_err)
}

#[pyfunction]
#[pyo3(signature = (svg_text, padding=0.0, precision=None, remove_dimensions=false))]
fn fit_viewbox_svg(svg_text: &str, padding: f64, precision: Option<usize>, remove_dimensions: bool) -> PyResult<String> {
    let metrics = svg_metrics_value(svg_text, None, 0.01);
    if let Some(err) = metrics.get("error").and_then(Value::as_str) {
        return Err(PyValueError::new_err(err.to_string()));
    }
    let Some(bbox) = metrics.get("bbox").and_then(Value::as_object) else {
        return Err(PyValueError::new_err("Cannot fit viewBox: SVG has no measurable geometry"));
    };
    let pad = padding.max(0.0);
    let values = [
        bbox.get("x").and_then(Value::as_f64).unwrap_or(0.0) - pad,
        bbox.get("y").and_then(Value::as_f64).unwrap_or(0.0) - pad,
        bbox.get("width").and_then(Value::as_f64).unwrap_or(0.0) + pad * 2.0,
        bbox.get("height").and_then(Value::as_f64).unwrap_or(0.0) + pad * 2.0,
    ];
    set_viewbox_svg_core(svg_text, &format_viewbox(values, None), precision, remove_dimensions).map_err(py_err)
}

fn parse_dimension(value: Option<&str>) -> Option<f64> {
    let number = value.and_then(|v| parse_float_list(v).first().copied())?;
    (number >= 0.0).then_some(number)
}

fn format_dimension(value: &str) -> String {
    if value.parse::<f64>().is_ok() {
        fmt_number(value.parse::<f64>().unwrap(), 6, false)
    } else {
        value.to_string()
    }
}

#[pyfunction]
fn resize_svg(svg_text: &str, width: Option<&str>, height: Option<&str>) -> PyResult<String> {
    let mut root = parse_svg_element(svg_text).map_err(py_err)?;
    if local_name(&root.name) != "svg" {
        return Err(PyValueError::new_err("Root element is not <svg>"));
    }
    if attr_value(&root, "viewBox").is_none() {
        if let (Some(w), Some(h)) = (parse_dimension(attr_value(&root, "width")), parse_dimension(attr_value(&root, "height"))) {
            root.attributes.insert("viewBox".to_string(), format_viewbox([0.0, 0.0, w, h], None));
        }
    }
    if let Some(width) = width {
        root.attributes.insert("width".to_string(), format_dimension(width));
    }
    if let Some(height) = height {
        root.attributes.insert("height".to_string(), format_dimension(height));
    }
    serialize_element(&root, false, 2).map_err(py_err)
}
