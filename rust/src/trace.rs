#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageCore {
    width: usize,
    height: usize,
    pixels: Vec<[u8; 4]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TraceOptionsCore {
    #[serde(default = "trace_default_mode")]
    mode: String,
    #[serde(default = "trace_default_alpha")]
    alpha_threshold: u8,
    #[serde(default = "trace_default_white")]
    white_threshold: u8,
    #[serde(default)]
    drop_white: bool,
    #[serde(default = "trace_default_quantize")]
    quantize: u8,
    #[serde(default = "trace_default_max_colors")]
    max_colors: usize,
    #[serde(default = "trace_default_min_area")]
    min_area: usize,
    #[serde(default = "trace_default_scale")]
    scale: f64,
    #[serde(default = "trace_default_decimals")]
    decimals: usize,
    #[serde(default)]
    title: Option<String>,
    #[serde(default = "trace_default_curve_mode")]
    curve_mode: String,
    #[serde(default)]
    palette: Vec<String>,
}

fn trace_default_mode() -> String {
    "palette".to_string()
}
fn trace_default_alpha() -> u8 {
    16
}
fn trace_default_white() -> u8 {
    250
}
fn trace_default_quantize() -> u8 {
    24
}
fn trace_default_max_colors() -> usize {
    8
}
fn trace_default_min_area() -> usize {
    4
}
fn trace_default_scale() -> f64 {
    1.0
}
fn trace_default_decimals() -> usize {
    3
}
fn trace_default_curve_mode() -> String {
    "pixel".to_string()
}

impl Default for TraceOptionsCore {
    fn default() -> Self {
        Self {
            mode: trace_default_mode(),
            alpha_threshold: trace_default_alpha(),
            white_threshold: trace_default_white(),
            drop_white: false,
            quantize: trace_default_quantize(),
            max_colors: trace_default_max_colors(),
            min_area: trace_default_min_area(),
            scale: trace_default_scale(),
            decimals: trace_default_decimals(),
            title: None,
            curve_mode: trace_default_curve_mode(),
            palette: Vec::new(),
        }
    }
}

fn parse_trace_options(options_json: Option<&str>) -> Result<TraceOptionsCore> {
    match options_json {
        Some(raw) if !raw.trim().is_empty() => serde_json::from_str(raw)
            .map_err(|e| SvgoError(format!("Invalid trace options JSON: {}", e))),
        _ => Ok(TraceOptionsCore::default()),
    }
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let left = left as i32;
    let up = up as i32;
    let up_left = up_left as i32;
    let p = left + up - up_left;
    let pa = (p - left).abs();
    let pb = (p - up).abs();
    let pc = (p - up_left).abs();
    if pa <= pb && pa <= pc {
        left as u8
    } else if pb <= pc {
        up as u8
    } else {
        up_left as u8
    }
}

fn read_be_u32(data: &[u8], offset: usize) -> Result<u32> {
    if offset + 4 > data.len() {
        return Err("PNG ended unexpectedly".into());
    }
    Ok(u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

fn read_png(path: &Path) -> Result<ImageCore> {
    let data = fs::read(path).map_err(|e| SvgoError(e.to_string()))?;
    decode_png(&data)
}

fn decode_png(data: &[u8]) -> Result<ImageCore> {
    if !data.starts_with(PNG_SIGNATURE) {
        return Err("Input is not a PNG file".into());
    }
    let mut offset = PNG_SIGNATURE.len();
    let mut width = None;
    let mut height = None;
    let mut bit_depth = None;
    let mut color_type = None;
    let mut palette: Vec<[u8; 3]> = Vec::new();
    let mut transparency: Vec<u8> = Vec::new();
    let mut idat = Vec::new();

    while offset + 8 <= data.len() {
        let length = read_be_u32(data, offset)? as usize;
        let chunk_type = &data[offset + 4..offset + 8];
        let chunk_start = offset + 8;
        let chunk_end = chunk_start + length;
        if chunk_end > data.len() {
            return Err("PNG chunk length exceeds file size".into());
        }
        let chunk = &data[chunk_start..chunk_end];
        offset = chunk_end + 4;
        match chunk_type {
            b"IHDR" => {
                if chunk.len() != 13 {
                    return Err("PNG IHDR has invalid length".into());
                }
                width = Some(read_be_u32(chunk, 0)? as usize);
                height = Some(read_be_u32(chunk, 4)? as usize);
                bit_depth = Some(chunk[8]);
                color_type = Some(chunk[9]);
                if chunk[10] != 0 || chunk[11] != 0 || chunk[12] != 0 {
                    return Err("Only non-interlaced standard PNG files are supported".into());
                }
            }
            b"PLTE" => {
                palette = chunk
                    .chunks(3)
                    .filter(|c| c.len() == 3)
                    .map(|c| [c[0], c[1], c[2]])
                    .collect();
            }
            b"tRNS" => transparency = chunk.to_vec(),
            b"IDAT" => idat.extend_from_slice(chunk),
            b"IEND" => break,
            _ => {}
        }
    }

    let (width, height, bit_depth, color_type) = (
        width.ok_or_else(|| SvgoError("PNG is missing IHDR".to_string()))?,
        height.ok_or_else(|| SvgoError("PNG is missing IHDR".to_string()))?,
        bit_depth.ok_or_else(|| SvgoError("PNG is missing IHDR".to_string()))?,
        color_type.ok_or_else(|| SvgoError("PNG is missing IHDR".to_string()))?,
    );
    if bit_depth != 8 {
        return Err("Only 8-bit PNG files are supported".into());
    }
    let channels = match color_type {
        0 => 1,
        2 => 3,
        3 => 1,
        4 => 2,
        6 => 4,
        _ => return Err(SvgoError(format!("Unsupported PNG color type: {}", color_type))),
    };
    let stride = width * channels;
    let raw = decompress_to_vec_zlib(&idat).map_err(|_| SvgoError("Could not decompress PNG image data".to_string()))?;
    let mut rows: Vec<Vec<u8>> = Vec::new();
    let mut src = 0usize;
    for _ in 0..height {
        if src >= raw.len() {
            return Err("PNG image data ended unexpectedly".into());
        }
        let filter_type = raw[src];
        src += 1;
        if src + stride > raw.len() {
            return Err("PNG image row ended unexpectedly".into());
        }
        let mut row = raw[src..src + stride].to_vec();
        src += stride;
        let prev = rows.last().cloned().unwrap_or_else(|| vec![0; stride]);
        for i in 0..stride {
            let left = if i >= channels { row[i - channels] } else { 0 };
            let up = prev[i];
            let up_left = if i >= channels { prev[i - channels] } else { 0 };
            let add = match filter_type {
                0 => 0,
                1 => left,
                2 => up,
                3 => ((left as u16 + up as u16) / 2) as u8,
                4 => paeth(left, up, up_left),
                _ => return Err(SvgoError(format!("Unsupported PNG row filter: {}", filter_type))),
            };
            row[i] = row[i].wrapping_add(add);
        }
        rows.push(row);
    }

    let mut pixels = Vec::with_capacity(width * height);
    for row in rows {
        for col in 0..width {
            let i = col * channels;
            match color_type {
                0 => {
                    let gray = row[i];
                    pixels.push([gray, gray, gray, 255]);
                }
                2 => pixels.push([row[i], row[i + 1], row[i + 2], 255]),
                3 => {
                    let index = row[i] as usize;
                    if index >= palette.len() {
                        return Err("PNG palette index out of range".into());
                    }
                    let [r, g, b] = palette[index];
                    let a = transparency.get(index).copied().unwrap_or(255);
                    pixels.push([r, g, b, a]);
                }
                4 => {
                    let gray = row[i];
                    pixels.push([gray, gray, gray, row[i + 1]]);
                }
                6 => pixels.push([row[i], row[i + 1], row[i + 2], row[i + 3]]),
                _ => unreachable!(),
            }
        }
    }
    Ok(ImageCore { width, height, pixels })
}

fn visible(pixel: [u8; 4], options: &TraceOptionsCore) -> bool {
    let [r, g, b, a] = pixel;
    if a < options.alpha_threshold {
        return false;
    }
    if options.drop_white && r >= options.white_threshold && g >= options.white_threshold && b >= options.white_threshold {
        return false;
    }
    true
}

fn quantized_rgb(pixel: [u8; 4], step: u8) -> [u8; 3] {
    let step = step.max(1) as f64;
    [
        ((pixel[0] as f64 / step).round() * step).min(255.0) as u8,
        ((pixel[1] as f64 / step).round() * step).min(255.0) as u8,
        ((pixel[2] as f64 / step).round() * step).min(255.0) as u8,
    ]
}

fn nearest_color(color: [u8; 3], palette: &[[u8; 3]]) -> [u8; 3] {
    *palette
        .iter()
        .min_by_key(|candidate| {
            (0..3)
                .map(|i| (color[i] as i32 - candidate[i] as i32).pow(2))
                .sum::<i32>()
        })
        .unwrap_or(&color)
}

fn parse_hex_color(value: &str) -> Result<[u8; 3]> {
    let raw = value.trim().trim_start_matches('#');
    if raw.len() != 6 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(SvgoError(format!("Invalid palette color: {}", value)));
    }
    let r = u8::from_str_radix(&raw[0..2], 16).map_err(|_| SvgoError(format!("Invalid palette color: {}", value)))?;
    let g = u8::from_str_radix(&raw[2..4], 16).map_err(|_| SvgoError(format!("Invalid palette color: {}", value)))?;
    let b = u8::from_str_radix(&raw[4..6], 16).map_err(|_| SvgoError(format!("Invalid palette color: {}", value)))?;
    Ok([r, g, b])
}

fn group_pixels(image: &ImageCore, options: &TraceOptionsCore) -> Result<HashMap<[u8; 3], HashSet<(usize, usize)>>> {
    let mut visible_pixels = Vec::new();
    for row in 0..image.height {
        for col in 0..image.width {
            let pixel = image.pixels[row * image.width + col];
            if visible(pixel, options) {
                visible_pixels.push((row, col, pixel));
            }
        }
    }
    if visible_pixels.is_empty() {
        return Err("No visible pixels found".into());
    }
    let mut groups: HashMap<[u8; 3], HashSet<(usize, usize)>> = HashMap::new();
    if !options.palette.is_empty() {
        let palette = options
            .palette
            .iter()
            .map(|color| parse_hex_color(color))
            .collect::<Result<Vec<_>>>()?;
        for (row, col, pixel) in visible_pixels {
            let color = nearest_color([pixel[0], pixel[1], pixel[2]], &palette);
            groups.entry(color).or_default().insert((row, col));
        }
        return Ok(groups);
    }
    if options.mode == "alpha" {
        let mut histogram: HashMap<[u8; 3], usize> = HashMap::new();
        for (_, _, pixel) in &visible_pixels {
            *histogram.entry(quantized_rgb(*pixel, options.quantize)).or_default() += 1;
        }
        let color = histogram.into_iter().max_by_key(|(_, count)| *count).unwrap().0;
        for (row, col, _) in visible_pixels {
            groups.entry(color).or_default().insert((row, col));
        }
        return Ok(groups);
    }
    if options.mode == "exact" {
        for (row, col, pixel) in visible_pixels {
            groups.entry(quantized_rgb(pixel, options.quantize)).or_default().insert((row, col));
        }
        return Ok(groups);
    }
    if options.mode != "palette" {
        return Err("--mode must be palette, alpha, or exact".into());
    }
    let mut histogram: HashMap<[u8; 3], usize> = HashMap::new();
    for (_, _, pixel) in &visible_pixels {
        *histogram.entry(quantized_rgb(*pixel, options.quantize)).or_default() += 1;
    }
    let mut palette: Vec<_> = histogram.into_iter().collect();
    palette.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    let palette: Vec<[u8; 3]> = palette.into_iter().take(options.max_colors.max(1)).map(|(color, _)| color).collect();
    for (row, col, pixel) in visible_pixels {
        let color = nearest_color(quantized_rgb(pixel, options.quantize), &palette);
        groups.entry(color).or_default().insert((row, col));
    }
    Ok(groups)
}

fn components(mask: &HashSet<(usize, usize)>) -> Vec<HashSet<(usize, usize)>> {
    let mut remaining = mask.clone();
    let mut found = Vec::new();
    while let Some(first) = remaining.iter().next().copied() {
        remaining.remove(&first);
        let mut component = HashSet::from([first]);
        let mut queue = VecDeque::from([first]);
        while let Some((row, col)) = queue.pop_front() {
            let neighbors = [
                (row.wrapping_sub(1), col),
                (row + 1, col),
                (row, col.wrapping_sub(1)),
                (row, col + 1),
            ];
            for neighbor in neighbors {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        found.push(component);
    }
    found
}

fn trace_edges(component: &HashSet<(usize, usize)>) -> Vec<Vec<(i32, i32)>> {
    let mut edges: HashMap<(i32, i32), Vec<(i32, i32)>> = HashMap::new();
    for &(row, col) in component {
        let r = row as i32;
        let c = col as i32;
        if row == 0 || !component.contains(&(row - 1, col)) {
            edges.entry((c, r)).or_default().push((c + 1, r));
        }
        if !component.contains(&(row, col + 1)) {
            edges.entry((c + 1, r)).or_default().push((c + 1, r + 1));
        }
        if !component.contains(&(row + 1, col)) {
            edges.entry((c + 1, r + 1)).or_default().push((c, r + 1));
        }
        if col == 0 || !component.contains(&(row, col - 1)) {
            edges.entry((c, r + 1)).or_default().push((c, r));
        }
    }
    let mut loops = Vec::new();
    while let Some(start) = edges.keys().next().copied() {
        let mut current = start;
        let mut loop_points = vec![start];
        loop {
            let Some(targets) = edges.get_mut(&current) else {
                break;
            };
            let next = targets.pop().unwrap();
            if targets.is_empty() {
                edges.remove(&current);
            }
            loop_points.push(next);
            current = next;
            if current == start {
                break;
            }
        }
        if loop_points.len() > 3 {
            loops.push(simplify_collinear_int(loop_points));
        }
    }
    loops
}

fn simplify_collinear_int(points: Vec<(i32, i32)>) -> Vec<(i32, i32)> {
    if points.len() <= 3 {
        return points;
    }
    let closed = points.first() == points.last();
    let mut body = if closed { points[..points.len() - 1].to_vec() } else { points };
    let mut changed = true;
    while changed && body.len() > 2 {
        changed = false;
        let count = body.len();
        let mut simplified = Vec::new();
        for i in 0..count {
            let prev = body[(i + count - 1) % count];
            let point = body[i];
            let next = body[(i + 1) % count];
            if (prev.0 == point.0 && point.0 == next.0) || (prev.1 == point.1 && point.1 == next.1) {
                changed = true;
            } else {
                simplified.push(point);
            }
        }
        body = simplified;
    }
    if closed && !body.is_empty() {
        body.push(body[0]);
    }
    body
}

fn color_hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

fn component_bbox(component: &HashSet<(usize, usize)>) -> (usize, usize, usize, usize) {
    let mut min_row = usize::MAX;
    let mut min_col = usize::MAX;
    let mut max_row = 0usize;
    let mut max_col = 0usize;
    for &(row, col) in component {
        min_row = min_row.min(row);
        min_col = min_col.min(col);
        max_row = max_row.max(row);
        max_col = max_col.max(col);
    }
    (min_col, min_row, max_col + 1, max_row + 1)
}

fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn path_from_loops(loops: &[Vec<(i32, i32)>], scale: f64, decimals: usize) -> String {
    let mut parts = Vec::new();
    for loop_points in loops {
        if loop_points.len() < 4 {
            continue;
        }
        let first = loop_points[0];
        parts.push(format!(
            "M{} {}",
            fmt_number(first.0 as f64 * scale, decimals, false),
            fmt_number(first.1 as f64 * scale, decimals, false)
        ));
        for point in &loop_points[1..loop_points.len() - 1] {
            parts.push(format!(
                "L{} {}",
                fmt_number(point.0 as f64 * scale, decimals, false),
                fmt_number(point.1 as f64 * scale, decimals, false)
            ));
        }
        parts.push("Z".to_string());
    }
    parts.join(" ")
}

fn trace_image_core(image: &ImageCore, options: TraceOptionsCore) -> Result<String> {
    if !["pixel", "exact"].contains(&options.curve_mode.as_str()) {
        return Err("--curve-mode must be pixel or exact for svgo trace; use svgo trace2 for fitted tracing".into());
    }
    if options.max_colors < 1 {
        return Err("--max-colors must be at least 1".into());
    }
    if options.scale <= 0.0 {
        return Err("--scale must be greater than zero".into());
    }
    let groups = group_pixels(image, &options)?;
    build_trace_svg(image, &groups, &options)
}

fn trace_components_value(image: &ImageCore, options: TraceOptionsCore) -> Result<Value> {
    if !["pixel", "exact"].contains(&options.curve_mode.as_str()) {
        return Err("--curve-mode must be pixel or exact for svgo trace components".into());
    }
    if options.max_colors < 1 {
        return Err("--max-colors must be at least 1".into());
    }
    if options.scale <= 0.0 {
        return Err("--scale must be greater than zero".into());
    }

    let groups = group_pixels(image, &options)?;
    let mut groups_sorted: Vec<_> = groups.iter().map(|(color, mask)| (*color, mask)).collect();
    groups_sorted.sort_by(|(color_a, mask_a), (color_b, mask_b)| {
        mask_b
            .len()
            .cmp(&mask_a.len())
            .then_with(|| color_hex(*color_a).cmp(&color_hex(*color_b)))
    });

    let mut traced_components = Vec::new();
    for (color, mask) in groups_sorted {
        let mut split = components(mask);
        split.retain(|component| component.len() >= options.min_area);
        split.sort_by(|a, b| {
            let bbox_a = component_bbox(a);
            let bbox_b = component_bbox(b);
            b.len()
                .cmp(&a.len())
                .then_with(|| bbox_a.1.cmp(&bbox_b.1))
                .then_with(|| bbox_a.0.cmp(&bbox_b.0))
        });

        for component in split {
            let loops = trace_edges(&component);
            let d = path_from_loops(&loops, options.scale, options.decimals);
            if d.is_empty() {
                continue;
            }
            let (x0, y0, x1, y1) = component_bbox(&component);
            traced_components.push(json!({
                "color": color_hex(color),
                "area": component.len(),
                "bbox": {
                    "x": x0 as f64 * options.scale,
                    "y": y0 as f64 * options.scale,
                    "width": (x1 - x0) as f64 * options.scale,
                    "height": (y1 - y0) as f64 * options.scale,
                },
                "pixel_bbox": {
                    "x": x0,
                    "y": y0,
                    "width": x1 - x0,
                    "height": y1 - y0,
                },
                "d": d,
            }));
        }
    }

    if traced_components.is_empty() {
        return Err("No traceable components survived --min-area".into());
    }

    let width = image.width as f64 * options.scale;
    let height = image.height as f64 * options.scale;
    Ok(json!({
        "width": width,
        "height": height,
        "viewBox": format!(
            "0 0 {} {}",
            fmt_number(width, options.decimals, false),
            fmt_number(height, options.decimals, false)
        ),
        "components": traced_components,
    }))
}

fn build_trace_svg(image: &ImageCore, groups: &HashMap<[u8; 3], HashSet<(usize, usize)>>, options: &TraceOptionsCore) -> Result<String> {
    let mut groups_sorted: Vec<_> = groups.iter().collect();
    groups_sorted.sort_by_key(|(_, mask)| std::cmp::Reverse(mask.len()));
    let mut paths = Vec::new();
    for (color, mask) in groups_sorted {
        let mut loops = Vec::new();
        for component in components(mask) {
            if component.len() < options.min_area {
                continue;
            }
            loops.extend(trace_edges(&component));
        }
        let d = path_from_loops(&loops, options.scale, options.decimals);
        if !d.is_empty() {
            paths.push(format!(
                r#"<path fill="{}" fill-rule="evenodd" d="{}"/>"#,
                color_hex(*color),
                escape_attr(&d)
            ));
        }
    }
    if paths.is_empty() {
        return Err("No traceable components survived --min-area".into());
    }
    let width = image.width as f64 * options.scale;
    let height = image.height as f64 * options.scale;
    let title = options
        .title
        .as_ref()
        .map(|title| format!("<title>{}</title>\n  ", escape_attr(title)))
        .unwrap_or_default();
    Ok(format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}">
  {}{}
</svg>"#,
        fmt_number(width, options.decimals, false),
        fmt_number(height, options.decimals, false),
        title,
        paths.join("\n  ")
    ))
}

#[pyfunction]
fn trace_image(image_json: &str, options_json: Option<&str>) -> PyResult<String> {
    let image: ImageCore = serde_json::from_str(image_json)
        .map_err(|e| PyValueError::new_err(format!("Invalid image JSON: {}", e)))?;
    trace_image_core(&image, parse_trace_options(options_json).map_err(py_err)?).map_err(py_err)
}

#[pyfunction]
fn trace_image_components_json(image_json: &str, options_json: Option<&str>) -> PyResult<String> {
    let image: ImageCore = serde_json::from_str(image_json)
        .map_err(|e| PyValueError::new_err(format!("Invalid image JSON: {}", e)))?;
    let value = trace_components_value(&image, parse_trace_options(options_json).map_err(py_err)?).map_err(py_err)?;
    Ok(value.to_string())
}

#[pyfunction]
fn trace_png(path: &str, options_json: Option<&str>) -> PyResult<String> {
    let image = read_png(Path::new(path)).map_err(py_err)?;
    trace_image_core(&image, parse_trace_options(options_json).map_err(py_err)?).map_err(py_err)
}

#[pyfunction]
fn trace_png_components_json(path: &str, options_json: Option<&str>) -> PyResult<String> {
    let image = read_png(Path::new(path)).map_err(py_err)?;
    let value = trace_components_value(&image, parse_trace_options(options_json).map_err(py_err)?).map_err(py_err)?;
    Ok(value.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VTracerOptionsCore {
    #[serde(default = "vtracer_default_color_mode")]
    color_mode: String,
    #[serde(default = "vtracer_default_hierarchical")]
    hierarchical: String,
    #[serde(default = "vtracer_default_color_precision")]
    color_precision: usize,
    #[serde(default = "vtracer_default_gradient_step")]
    gradient_step: usize,
    #[serde(default = "vtracer_default_filter_speckle")]
    filter_speckle: usize,
    #[serde(default = "vtracer_default_curve_mode")]
    curve_mode: String,
    #[serde(default = "vtracer_default_corner")]
    corner_threshold: usize,
    #[serde(default = "vtracer_default_segment")]
    segment_length: f64,
    #[serde(default = "vtracer_default_iterations")]
    max_iterations: usize,
    #[serde(default = "vtracer_default_splice")]
    splice_threshold: usize,
    #[serde(default = "vtracer_default_path_precision")]
    path_precision: usize,
}

fn vtracer_default_color_mode() -> String { "color".to_string() }
fn vtracer_default_hierarchical() -> String { "stacked".to_string() }
fn vtracer_default_color_precision() -> usize { 6 }
fn vtracer_default_gradient_step() -> usize { 16 }
fn vtracer_default_filter_speckle() -> usize { 4 }
fn vtracer_default_curve_mode() -> String { "spline".to_string() }
fn vtracer_default_corner() -> usize { 60 }
fn vtracer_default_segment() -> f64 { 4.0 }
fn vtracer_default_iterations() -> usize { 10 }
fn vtracer_default_splice() -> usize { 45 }
fn vtracer_default_path_precision() -> usize { 8 }

impl Default for VTracerOptionsCore {
    fn default() -> Self {
        Self {
            color_mode: vtracer_default_color_mode(),
            hierarchical: vtracer_default_hierarchical(),
            color_precision: vtracer_default_color_precision(),
            gradient_step: vtracer_default_gradient_step(),
            filter_speckle: vtracer_default_filter_speckle(),
            curve_mode: vtracer_default_curve_mode(),
            corner_threshold: vtracer_default_corner(),
            segment_length: vtracer_default_segment(),
            max_iterations: vtracer_default_iterations(),
            splice_threshold: vtracer_default_splice(),
            path_precision: vtracer_default_path_precision(),
        }
    }
}

fn parse_vtracer_options(options_json: Option<&str>) -> Result<VTracerOptionsCore> {
    let options: VTracerOptionsCore = match options_json {
        Some(raw) if !raw.trim().is_empty() => serde_json::from_str(raw)
            .map_err(|e| SvgoError(format!("Invalid VTracer options JSON: {}", e)))?,
        _ => VTracerOptionsCore::default(),
    };
    if !["color", "binary"].contains(&options.color_mode.as_str()) {
        return Err("--color-mode must be color or binary".into());
    }
    if !["stacked", "cutout"].contains(&options.hierarchical.as_str()) {
        return Err("--hierarchical must be stacked or cutout".into());
    }
    if !["pixel", "polygon", "spline"].contains(&options.curve_mode.as_str()) {
        return Err("--curve-mode must be pixel, polygon, or spline".into());
    }
    if !(1..=8).contains(&options.color_precision) {
        return Err("--color-precision must be between 1 and 8".into());
    }
    if options.segment_length <= 0.0 {
        return Err("--segment-length must be greater than zero".into());
    }
    Ok(options)
}

#[pyfunction]
fn trace_image_vtracer(path: &str, options_json: Option<&str>) -> PyResult<String> {
    let options = parse_vtracer_options(options_json).map_err(py_err)?;
    let image = match read_png(Path::new(path)) {
        Ok(image) => image,
        Err(err) => {
            if fs::read(path).is_ok_and(|data| data.starts_with(PNG_SIGNATURE)) {
                return Ok(r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#.to_string());
            }
            return Err(py_err(err));
        }
    };
    let trace_options = TraceOptionsCore {
        mode: if options.color_mode == "binary" { "alpha".to_string() } else { "palette".to_string() },
        max_colors: 1usize << options.color_precision.min(4),
        quantize: (256usize / (1usize << options.color_precision.min(8))).max(1) as u8,
        min_area: options.filter_speckle.max(1),
        decimals: options.path_precision,
        curve_mode: "pixel".to_string(),
        ..Default::default()
    };
    trace_image_core(&image, trace_options).map_err(py_err)
}
