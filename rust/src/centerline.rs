#[derive(Debug, Clone, Serialize, Deserialize)]
struct CenterlineOptionsCore {
    #[serde(default = "center_default_emit")]
    emit: String,
    #[serde(default = "center_default_mode")]
    mode: String,
    #[serde(default = "center_default_scale")]
    scale: f64,
    #[serde(default = "center_default_max_size")]
    max_size: usize,
    #[serde(default = "center_default_curve_samples")]
    curve_samples: usize,
    #[serde(default = "center_default_simplify")]
    simplify: f64,
    #[serde(default = "center_default_min_length")]
    min_length: f64,
    #[serde(default = "center_default_stroke_width")]
    stroke_width: String,
    #[serde(default = "center_default_linecap")]
    linecap: String,
    #[serde(default = "center_default_linejoin")]
    linejoin: String,
    #[serde(default = "center_default_decimals")]
    decimals: usize,
    #[serde(default)]
    polyline: bool,
    #[serde(default = "center_default_fill_rule")]
    fill_rule: String,
    #[serde(default = "center_default_svg_paths")]
    svg_paths: String,
    #[serde(default)]
    keep_failed: bool,
    #[serde(default)]
    bridge_gap: f64,
}

fn center_default_emit() -> String { "path".to_string() }
fn center_default_mode() -> String { "longest".to_string() }
fn center_default_scale() -> f64 { 2.0 }
fn center_default_max_size() -> usize { 1600 }
fn center_default_curve_samples() -> usize { 24 }
fn center_default_simplify() -> f64 { 6.0 }
fn center_default_min_length() -> f64 { 20.0 }
fn center_default_stroke_width() -> String { "auto".to_string() }
fn center_default_linecap() -> String { "round".to_string() }
fn center_default_linejoin() -> String { "round".to_string() }
fn center_default_decimals() -> usize { 3 }
fn center_default_fill_rule() -> String { "evenodd".to_string() }
fn center_default_svg_paths() -> String { "first".to_string() }

impl Default for CenterlineOptionsCore {
    fn default() -> Self {
        Self {
            emit: center_default_emit(),
            mode: center_default_mode(),
            scale: center_default_scale(),
            max_size: center_default_max_size(),
            curve_samples: center_default_curve_samples(),
            simplify: center_default_simplify(),
            min_length: center_default_min_length(),
            stroke_width: center_default_stroke_width(),
            linecap: center_default_linecap(),
            linejoin: center_default_linejoin(),
            decimals: center_default_decimals(),
            polyline: false,
            fill_rule: center_default_fill_rule(),
            svg_paths: center_default_svg_paths(),
            keep_failed: false,
            bridge_gap: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
struct RasterContext {
    min_x: f64,
    min_y: f64,
    scale: f64,
    pad: usize,
    width: usize,
    height: usize,
}

fn parse_center_options(options_json: Option<&str>) -> Result<CenterlineOptionsCore> {
    match options_json {
        Some(raw) if !raw.trim().is_empty() => serde_json::from_str(raw)
            .map_err(|e| SvgoError(format!("Invalid centerline options JSON: {}", e))),
        _ => Ok(CenterlineOptionsCore::default()),
    }
}

fn quad_point(p0: Point, p1: Point, p2: Point, t: f64) -> Point {
    let mt = 1.0 - t;
    Point::new(
        mt.powi(2) * p0.x + 2.0 * mt * t * p1.x + t.powi(2) * p2.x,
        mt.powi(2) * p0.y + 2.0 * mt * t * p1.y + t.powi(2) * p2.y,
    )
}

fn arc_points(
    p0: Point,
    rx: f64,
    ry: f64,
    x_axis_rotation: f64,
    large_arc: i32,
    sweep: i32,
    p1: Point,
    curve_samples: usize,
) -> Vec<Point> {
    let Ok((center, rx, ry, phi, theta1, delta)) =
        arc_to_center(p0, rx, ry, x_axis_rotation, large_arc, sweep, p1)
    else {
        return vec![p1];
    };
    let cos_phi = phi.cos();
    let sin_phi = phi.sin();
    let approx_length = rx.max(ry) * delta.abs();
    let n = 4usize.max(240usize.min((approx_length / 8.0).ceil() as usize).min(curve_samples * 4));
    (1..=n)
        .map(|step| {
            let theta = theta1 + delta * (step as f64 / n as f64);
            Point::new(
                cos_phi * rx * theta.cos() - sin_phi * ry * theta.sin() + center.x,
                sin_phi * rx * theta.cos() + cos_phi * ry * theta.sin() + center.y,
            )
        })
        .collect()
}

fn sample_count(points: &[Point], samples: usize) -> usize {
    let length: f64 = points.windows(2).map(|w| distance(w[0], w[1])).sum();
    4usize.max(160usize.min((length / 8.0).ceil() as usize).min(samples))
}

fn append_flat_point(subpath: &mut Vec<Point>, point: Point) {
    if subpath.last().is_none_or(|last| distance(*last, point) > 1e-9) {
        subpath.push(point);
    }
}

fn flatten_path(path_data: &str, curve_samples: usize) -> Result<Vec<Vec<Point>>> {
    let tokens = tokenize_path(path_data);
    if tokens.is_empty() {
        return Err("No SVG path tokens found".into());
    }
    let mut stream = TokenStream::new(tokens);
    let mut subpaths = Vec::new();
    let mut subpath = Vec::new();
    let mut command = String::new();
    let mut current = Point::new(0.0, 0.0);
    let mut start = Point::new(0.0, 0.0);
    let mut last_cubic_ctrl: Option<Point> = None;
    let mut last_quad_ctrl: Option<Point> = None;

    while stream.has_more() {
        if stream.peek().is_some_and(is_command_token) {
            command = stream.next()?;
        } else if command.is_empty() {
            return Err("Path data must begin with a command".into());
        }
        let upper = command.to_ascii_uppercase();
        let relative = command.chars().next().is_some_and(|c| c.is_ascii_lowercase());
        let read_point = |stream: &mut TokenStream, current: Point| -> Result<Point> {
            let x = stream.number("x")?;
            let y = stream.number("y")?;
            Ok(if relative { Point::new(current.x + x, current.y + y) } else { Point::new(x, y) })
        };
        match upper.as_str() {
            "M" => {
                if !stream.has_numbers(2) {
                    return Err("M command requires x y".into());
                }
                if !subpath.is_empty() {
                    subpaths.push(subpath);
                    subpath = Vec::new();
                }
                current = read_point(&mut stream, current)?;
                start = current;
                append_flat_point(&mut subpath, current);
                while stream.has_numbers(2) {
                    current = read_point(&mut stream, current)?;
                    append_flat_point(&mut subpath, current);
                }
                command = if relative { "l" } else { "L" }.to_string();
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            "L" => {
                while stream.has_numbers(2) {
                    current = read_point(&mut stream, current)?;
                    append_flat_point(&mut subpath, current);
                }
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            "H" => {
                while stream.has_numbers(1) {
                    let x = stream.number("x")?;
                    current = if relative { Point::new(current.x + x, current.y) } else { Point::new(x, current.y) };
                    append_flat_point(&mut subpath, current);
                }
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            "V" => {
                while stream.has_numbers(1) {
                    let y = stream.number("y")?;
                    current = if relative { Point::new(current.x, current.y + y) } else { Point::new(current.x, y) };
                    append_flat_point(&mut subpath, current);
                }
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            "C" => {
                while stream.has_numbers(6) {
                    let p0 = current;
                    let p1 = read_point(&mut stream, current)?;
                    let p2 = read_point(&mut stream, current)?;
                    let p3 = read_point(&mut stream, current)?;
                    let n = sample_count(&[p0, p1, p2, p3], curve_samples);
                    for step in 1..=n {
                        append_flat_point(&mut subpath, cubic_point(p0, p1, p2, p3, step as f64 / n as f64));
                    }
                    current = p3;
                    last_cubic_ctrl = Some(p2);
                    last_quad_ctrl = None;
                }
            }
            "S" => {
                while stream.has_numbers(4) {
                    let p0 = current;
                    let p1 = last_cubic_ctrl.map(|c| Point::new(2.0 * current.x - c.x, 2.0 * current.y - c.y)).unwrap_or(current);
                    let p2 = read_point(&mut stream, current)?;
                    let p3 = read_point(&mut stream, current)?;
                    let n = sample_count(&[p0, p1, p2, p3], curve_samples);
                    for step in 1..=n {
                        append_flat_point(&mut subpath, cubic_point(p0, p1, p2, p3, step as f64 / n as f64));
                    }
                    current = p3;
                    last_cubic_ctrl = Some(p2);
                    last_quad_ctrl = None;
                }
            }
            "Q" => {
                while stream.has_numbers(4) {
                    let p0 = current;
                    let p1 = read_point(&mut stream, current)?;
                    let p2 = read_point(&mut stream, current)?;
                    let n = sample_count(&[p0, p1, p2], curve_samples);
                    for step in 1..=n {
                        append_flat_point(&mut subpath, quad_point(p0, p1, p2, step as f64 / n as f64));
                    }
                    current = p2;
                    last_quad_ctrl = Some(p1);
                    last_cubic_ctrl = None;
                }
            }
            "T" => {
                while stream.has_numbers(2) {
                    let p0 = current;
                    let p1 = last_quad_ctrl.map(|c| Point::new(2.0 * current.x - c.x, 2.0 * current.y - c.y)).unwrap_or(current);
                    let p2 = read_point(&mut stream, current)?;
                    let n = sample_count(&[p0, p1, p2], curve_samples);
                    for step in 1..=n {
                        append_flat_point(&mut subpath, quad_point(p0, p1, p2, step as f64 / n as f64));
                    }
                    current = p2;
                    last_quad_ctrl = Some(p1);
                    last_cubic_ctrl = None;
                }
            }
            "Z" => {
                append_flat_point(&mut subpath, start);
                current = start;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            "A" => {
                while stream.has_more() && stream.peek().is_some_and(|t| !is_command_token(t)) {
                    let p0 = current;
                    let rx = stream.number("rx")?;
                    let ry = stream.number("ry")?;
                    let rotation = stream.number("x-axis-rotation")?;
                    let large_arc = stream.flag("large-arc-flag")?;
                    let sweep = stream.flag("sweep-flag")?;
                    let endpoint = read_point(&mut stream, current)?;
                    for point in arc_points(p0, rx, ry, rotation, large_arc, sweep, endpoint, curve_samples) {
                        append_flat_point(&mut subpath, point);
                    }
                    current = endpoint;
                }
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            _ => return Err(SvgoError(format!("Unsupported path command: {}", command))),
        }
    }
    if !subpath.is_empty() {
        subpaths.push(subpath);
    }
    if subpaths.is_empty() {
        return Err("Path did not produce any drawable subpaths".into());
    }
    Ok(subpaths)
}

fn bounds_points(subpaths: &[Vec<Point>]) -> (f64, f64, f64, f64) {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in subpaths.iter().flatten() {
        min_x = min_x.min(point.x);
        min_y = min_y.min(point.y);
        max_x = max_x.max(point.x);
        max_y = max_y.max(point.y);
    }
    (min_x, min_y, max_x, max_y)
}

fn make_context(subpaths: &[Vec<Point>], mut scale: f64, max_size: usize) -> Result<RasterContext> {
    let (min_x, min_y, max_x, max_y) = bounds_points(subpaths);
    if scale <= 0.0 {
        return Err("--scale must be greater than zero".into());
    }
    let svg_width = (max_x - min_x).max(1.0);
    let svg_height = (max_y - min_y).max(1.0);
    if max_size > 0 {
        scale = scale.min(max_size as f64 / svg_width.max(svg_height));
    }
    let pad = 8usize.max((8.0 * scale).ceil() as usize);
    let width = (svg_width * scale).ceil() as usize + pad * 2 + 2;
    let height = (svg_height * scale).ceil() as usize + pad * 2 + 2;
    Ok(RasterContext { min_x, min_y, scale, pad, width, height })
}

fn to_pixel(point: Point, ctx: RasterContext) -> (f64, f64) {
    ((point.x - ctx.min_x) * ctx.scale + ctx.pad as f64, (point.y - ctx.min_y) * ctx.scale + ctx.pad as f64)
}

fn to_svg_point(pixel: (usize, usize), ctx: RasterContext) -> Point {
    let (row, col) = pixel;
    Point::new(
        (col as f64 + 0.5 - ctx.pad as f64) / ctx.scale + ctx.min_x,
        (row as f64 + 0.5 - ctx.pad as f64) / ctx.scale + ctx.min_y,
    )
}

fn rasterize(subpaths: &[Vec<Point>], ctx: RasterContext) -> Result<HashSet<(usize, usize)>> {
    let pixel_paths: Vec<Vec<(f64, f64)>> = subpaths
        .iter()
        .filter(|subpath| subpath.len() >= 2)
        .map(|subpath| subpath.iter().map(|p| to_pixel(*p, ctx)).collect())
        .collect();
    let mut filled = HashSet::new();
    for row in 0..ctx.height {
        let y = row as f64 + 0.5;
        let mut intersections = Vec::new();
        for points in &pixel_paths {
            for pair in points.windows(2) {
                let (x1, y1) = pair[0];
                let (x2, y2) = pair[1];
                if (y1 - y2).abs() <= f64::EPSILON {
                    continue;
                }
                if (y1 <= y && y < y2) || (y2 <= y && y < y1) {
                    let t = (y - y1) / (y2 - y1);
                    intersections.push(x1 + t * (x2 - x1));
                }
            }
        }
        intersections.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
        for pair in intersections.chunks(2) {
            if pair.len() != 2 {
                continue;
            }
            let start = 0isize.max((pair[0] - 0.5).ceil() as isize) as usize;
            let end = (ctx.width as isize - 1).min((pair[1] - 0.5).floor() as isize);
            if end >= start as isize {
                for col in start..=end as usize {
                    filled.insert((row, col));
                }
            }
        }
    }
    if filled.is_empty() {
        return Err("Rasterization produced an empty mask".into());
    }
    Ok(filled)
}

const NEIGHBORS: [(isize, isize); 8] = [(-1, 0), (-1, 1), (0, 1), (1, 1), (1, 0), (1, -1), (0, -1), (-1, -1)];

fn add_neighbor(pixel: (usize, usize), delta: (isize, isize), height: usize, width: usize) -> Option<(usize, usize)> {
    let row = pixel.0 as isize + delta.0;
    let col = pixel.1 as isize + delta.1;
    if row >= 0 && col >= 0 && row < height as isize && col < width as isize {
        Some((row as usize, col as usize))
    } else {
        None
    }
}

fn zhang_suen_thin(mask: &HashSet<(usize, usize)>, height: usize, width: usize) -> Result<HashSet<(usize, usize)>> {
    let mut foreground = mask.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for step in 0..=1 {
            let mut remove = Vec::new();
            for &(row, col) in &foreground {
                if row == 0 || col == 0 || row >= height - 1 || col >= width - 1 {
                    continue;
                }
                let vals: Vec<i32> = NEIGHBORS
                    .iter()
                    .map(|d| add_neighbor((row, col), *d, height, width).is_some_and(|p| foreground.contains(&p)) as i32)
                    .collect();
                let b: i32 = vals.iter().sum();
                if !(2..=6).contains(&b) {
                    continue;
                }
                let mut seq = vals.clone();
                seq.push(vals[0]);
                let a = (0..8).filter(|i| seq[*i] == 0 && seq[*i + 1] == 1).count();
                if a != 1 {
                    continue;
                }
                let p2 = vals[0];
                let p4 = vals[2];
                let p6 = vals[4];
                let p8 = vals[6];
                if step == 0 {
                    if p2 * p4 * p6 != 0 || p4 * p6 * p8 != 0 {
                        continue;
                    }
                } else if p2 * p4 * p8 != 0 || p2 * p6 * p8 != 0 {
                    continue;
                }
                remove.push((row, col));
            }
            if !remove.is_empty() {
                for pixel in remove {
                    foreground.remove(&pixel);
                }
                changed = true;
            }
        }
    }
    if foreground.is_empty() {
        return Err("Skeletonization produced an empty centerline".into());
    }
    Ok(foreground)
}

fn chamfer_distance(mask: &HashSet<(usize, usize)>, height: usize, width: usize) -> HashMap<(usize, usize), f64> {
    let inf = 1e12;
    let mut dist = vec![vec![0.0_f64; width]; height];
    for row in 0..height {
        for col in 0..width {
            dist[row][col] = if mask.contains(&(row, col)) { inf } else { 0.0 };
        }
    }
    let root2 = 2.0_f64.sqrt();
    for row in 0..height {
        for col in 0..width {
            if dist[row][col] == 0.0 {
                continue;
            }
            let mut best = dist[row][col];
            if row > 0 {
                best = best.min(dist[row - 1][col] + 1.0);
                if col > 0 {
                    best = best.min(dist[row - 1][col - 1] + root2);
                }
                if col + 1 < width {
                    best = best.min(dist[row - 1][col + 1] + root2);
                }
            }
            if col > 0 {
                best = best.min(dist[row][col - 1] + 1.0);
            }
            dist[row][col] = best;
        }
    }
    for row in (0..height).rev() {
        for col in (0..width).rev() {
            if dist[row][col] == 0.0 {
                continue;
            }
            let mut best = dist[row][col];
            if row + 1 < height {
                best = best.min(dist[row + 1][col] + 1.0);
                if col > 0 {
                    best = best.min(dist[row + 1][col - 1] + root2);
                }
                if col + 1 < width {
                    best = best.min(dist[row + 1][col + 1] + root2);
                }
            }
            if col + 1 < width {
                best = best.min(dist[row][col + 1] + 1.0);
            }
            dist[row][col] = best;
        }
    }
    mask.iter().map(|p| (*p, dist[p.0][p.1])).collect()
}

fn skeleton_neighbors(pixel: (usize, usize), skeleton: &HashSet<(usize, usize)>) -> Vec<(usize, usize)> {
    NEIGHBORS
        .iter()
        .filter_map(|(dr, dc)| {
            let row = pixel.0 as isize + dr;
            let col = pixel.1 as isize + dc;
            (row >= 0 && col >= 0).then_some((row as usize, col as usize))
        })
        .filter(|p| skeleton.contains(p))
        .collect()
}

fn connected_components_skeleton(skeleton: &HashSet<(usize, usize)>) -> Vec<HashSet<(usize, usize)>> {
    let mut remaining = skeleton.clone();
    let mut components = Vec::new();
    while let Some(first) = remaining.iter().next().copied() {
        remaining.remove(&first);
        let mut component = HashSet::from([first]);
        let mut queue = VecDeque::from([first]);
        while let Some(pixel) = queue.pop_front() {
            for neighbor in skeleton_neighbors(pixel, skeleton) {
                if remaining.remove(&neighbor) {
                    component.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        components.push(component);
    }
    components
}

fn node_clusters(nodes: &HashSet<(usize, usize)>) -> (Vec<HashSet<(usize, usize)>>, HashMap<(usize, usize), usize>) {
    let mut remaining = nodes.clone();
    let mut clusters = Vec::new();
    let mut node_to_cluster = HashMap::new();
    while let Some(first) = remaining.iter().next().copied() {
        remaining.remove(&first);
        let cluster_id = clusters.len();
        let mut cluster = HashSet::from([first]);
        let mut queue = VecDeque::from([first]);
        while let Some(pixel) = queue.pop_front() {
            for delta in NEIGHBORS {
                let row = pixel.0 as isize + delta.0;
                let col = pixel.1 as isize + delta.1;
                if row < 0 || col < 0 {
                    continue;
                }
                let neighbor = (row as usize, col as usize);
                if remaining.remove(&neighbor) {
                    cluster.insert(neighbor);
                    queue.push_back(neighbor);
                }
            }
        }
        for pixel in &cluster {
            node_to_cluster.insert(*pixel, cluster_id);
        }
        clusters.push(cluster);
    }
    (clusters, node_to_cluster)
}

fn cluster_representative(cluster: &HashSet<(usize, usize)>) -> (usize, usize) {
    let row_mean = cluster.iter().map(|p| p.0 as f64).sum::<f64>() / cluster.len() as f64;
    let col_mean = cluster.iter().map(|p| p.1 as f64).sum::<f64>() / cluster.len() as f64;
    cluster
        .iter()
        .copied()
        .min_by(|a, b| {
            let da = (a.0 as f64 - row_mean).hypot(a.1 as f64 - col_mean);
            let db = (b.0 as f64 - row_mean).hypot(b.1 as f64 - col_mean);
            da.partial_cmp(&db)
                .unwrap_or(Ordering::Equal)
                .then_with(|| a.cmp(b))
        })
        .unwrap()
}

fn skeleton_component_endpoints(component: &HashSet<(usize, usize)>) -> Vec<(usize, usize)> {
    let endpoints: Vec<_> = component
        .iter()
        .copied()
        .filter(|pixel| skeleton_neighbors(*pixel, component).len() <= 1)
        .collect();
    if endpoints.is_empty() && component.len() == 1 {
        component.iter().copied().collect()
    } else {
        endpoints
    }
}

fn draw_pixel_line(a: (usize, usize), b: (usize, usize), height: usize, width: usize) -> Vec<(usize, usize)> {
    let (mut y0, mut x0) = (a.0 as isize, a.1 as isize);
    let (y1, x1) = (b.0 as isize, b.1 as isize);
    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    let mut points = Vec::new();
    loop {
        if y0 >= 0 && x0 >= 0 && y0 < height as isize && x0 < width as isize {
            points.push((y0 as usize, x0 as usize));
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
    points
}

fn bridge_skeleton_gaps(skeleton: &HashSet<(usize, usize)>, height: usize, width: usize, max_gap_px: f64) -> HashSet<(usize, usize)> {
    if max_gap_px <= 0.0 {
        return skeleton.clone();
    }
    let mut bridged = skeleton.clone();
    for _ in 0..128 {
        let components = connected_components_skeleton(&bridged);
        if components.len() < 2 {
            break;
        }
        let endpoints: Vec<Vec<(usize, usize)>> = components.iter().map(skeleton_component_endpoints).collect();
        let mut best: Option<(f64, (usize, usize), (usize, usize))> = None;
        for i in 0..components.len() {
            if endpoints[i].is_empty() {
                continue;
            }
            for j in i + 1..components.len() {
                if endpoints[j].is_empty() {
                    continue;
                }
                for &a in &endpoints[i] {
                    for &b in &endpoints[j] {
                        let dist = (a.0 as f64 - b.0 as f64).hypot(a.1 as f64 - b.1 as f64);
                        if dist <= max_gap_px && best.is_none_or(|(best_dist, _, _)| dist < best_dist) {
                            best = Some((dist, a, b));
                        }
                    }
                }
            }
        }
        let Some((_dist, a, b)) = best else {
            break;
        };
        let before = bridged.len();
        for pixel in draw_pixel_line(a, b, height, width) {
            bridged.insert(pixel);
        }
        if bridged.len() == before {
            break;
        }
    }
    bridged
}

fn farthest_path(component: &HashSet<(usize, usize)>, start: (usize, usize)) -> Vec<(usize, usize)> {
    fn bfs(component: &HashSet<(usize, usize)>, origin: (usize, usize)) -> ((usize, usize), HashMap<(usize, usize), Option<(usize, usize)>>) {
        let mut parents = HashMap::from([(origin, None)]);
        let mut queue = VecDeque::from([origin]);
        let mut last = origin;
        while let Some(pixel) = queue.pop_front() {
            last = pixel;
            for neighbor in skeleton_neighbors(pixel, component) {
                if let std::collections::hash_map::Entry::Vacant(entry) = parents.entry(neighbor) {
                    entry.insert(Some(pixel));
                    queue.push_back(neighbor);
                }
            }
        }
        (last, parents)
    }
    let (a, _) = bfs(component, start);
    let (b, parents) = bfs(component, a);
    let mut path = vec![b];
    while let Some(Some(parent)) = parents.get(path.last().unwrap()) {
        path.push(*parent);
    }
    path.reverse();
    path
}

fn cycle_path(component: &HashSet<(usize, usize)>) -> Vec<(usize, usize)> {
    let Some(start) = component.iter().copied().min() else {
        return Vec::new();
    };
    let mut neighbors = skeleton_neighbors(start, component);
    neighbors.sort();
    let Some(mut current) = neighbors.first().copied() else {
        return vec![start];
    };

    let mut path = vec![start];
    let mut visited = HashSet::from([start]);
    let mut previous = start;
    for _ in 0..component.len() + 2 {
        path.push(current);
        if current == start {
            break;
        }
        visited.insert(current);
        let mut candidates: Vec<_> = skeleton_neighbors(current, component)
            .into_iter()
            .filter(|pixel| *pixel != previous)
            .collect();
        candidates.sort();
        if candidates.is_empty() {
            break;
        }
        let next = candidates
            .iter()
            .copied()
            .find(|pixel| !visited.contains(pixel))
            .or_else(|| {
                if visited.len() >= component.len() {
                    candidates.iter().copied().find(|pixel| *pixel == start)
                } else {
                    None
                }
            })
            .unwrap_or(candidates[0]);
        previous = current;
        current = next;
    }
    if path.last().is_some_and(|last| *last != start) && visited.len() >= component.len() {
        path.push(start);
    }
    path
}

fn pixel_path_length(path: &[(usize, usize)]) -> f64 {
    path.windows(2)
        .map(|w| (w[1].0 as f64 - w[0].0 as f64).hypot(w[1].1 as f64 - w[0].1 as f64))
        .sum()
}

fn trace_skeleton(skeleton: &HashSet<(usize, usize)>, mode: &str, min_length_px: f64) -> Vec<Vec<(usize, usize)>> {
    let mut paths = Vec::new();
    for component in connected_components_skeleton(skeleton) {
        let endpoints: Vec<_> = component
            .iter()
            .copied()
            .filter(|pixel| skeleton_neighbors(*pixel, &component).len() <= 1)
            .collect();
        let starts = if endpoints.is_empty() { vec![*component.iter().next().unwrap()] } else { endpoints };
        if mode == "longest" {
            paths.push(farthest_path(&component, starts[0]));
        } else {
            let mut used_edges: HashSet<[(usize, usize); 2]> = HashSet::new();
            let nodes: HashSet<_> = component
                .iter()
                .copied()
                .filter(|pixel| skeleton_neighbors(*pixel, &component).len() != 2)
                .collect();
            if nodes.is_empty() {
                paths.push(cycle_path(&component));
                continue;
            }
            let (clusters, node_to_cluster) = node_clusters(&nodes);
            let cluster_reps: Vec<_> = clusters.iter().map(cluster_representative).collect();
            for (cluster_id, cluster) in clusters.iter().enumerate() {
                let mut cluster_nodes: Vec<_> = cluster.iter().copied().collect();
                cluster_nodes.sort();
                for node in cluster_nodes {
                    let mut neighbors = skeleton_neighbors(node, &component);
                    neighbors.sort();
                    for neighbor in neighbors {
                        if node_to_cluster.get(&neighbor).is_some_and(|id| *id == cluster_id) {
                            continue;
                        }
                    let mut edge = [node, neighbor];
                    edge.sort();
                    if used_edges.contains(&edge) {
                        continue;
                    }
                    let mut chain = vec![cluster_reps[cluster_id], node, neighbor];
                    used_edges.insert(edge);
                    let mut prev = node;
                    let mut current = neighbor;
                    while !node_to_cluster.contains_key(&current) {
                        let mut candidates: Vec<_> = skeleton_neighbors(current, &component).into_iter().filter(|p| *p != prev).collect();
                        candidates.sort();
                        let Some(next) = candidates.first().copied() else {
                            break;
                        };
                        let mut edge = [current, next];
                        edge.sort();
                        used_edges.insert(edge);
                        chain.push(next);
                        prev = current;
                        current = next;
                    }
                    if let Some(target_cluster_id) = node_to_cluster.get(&current).copied() {
                        let target_rep = cluster_reps[target_cluster_id];
                        if chain.last().is_some_and(|last| *last != target_rep) {
                            chain.push(target_rep);
                        }
                    }
                    paths.push(chain);
                }
            }
            }
        }
    }
    let mut filtered: Vec<_> = paths
        .iter()
        .filter(|path| pixel_path_length(path) >= min_length_px)
        .cloned()
        .collect();
    if filtered.is_empty() {
        paths.sort_by(|a, b| pixel_path_length(b).partial_cmp(&pixel_path_length(a)).unwrap_or(Ordering::Equal));
        filtered = paths.into_iter().take(1).collect();
    }
    filtered.sort_by(|a, b| pixel_path_length(b).partial_cmp(&pixel_path_length(a)).unwrap_or(Ordering::Equal));
    if mode == "longest" {
        filtered.truncate(1);
    }
    filtered
}

fn point_line_distance(point: Point, start: Point, end: Point) -> f64 {
    let vx = end.x - start.x;
    let vy = end.y - start.y;
    let wx = point.x - start.x;
    let wy = point.y - start.y;
    let denom = vx * vx + vy * vy;
    if denom == 0.0 {
        return distance(point, start);
    }
    let t = ((wx * vx + wy * vy) / denom).clamp(0.0, 1.0);
    let px = start.x + t * vx;
    let py = start.y + t * vy;
    ((point.x - px).powi(2) + (point.y - py).powi(2)).sqrt()
}

fn simplify_points(points: &[Point], tolerance: f64) -> Vec<Point> {
    if points.len() <= 2 || tolerance <= 0.0 {
        return points.to_vec();
    }
    let start = points[0];
    let end = *points.last().unwrap();
    let mut max_distance = -1.0;
    let mut index = 0usize;
    for (i, point) in points.iter().enumerate().take(points.len() - 1).skip(1) {
        let dist = point_line_distance(*point, start, end);
        if dist > max_distance {
            max_distance = dist;
            index = i;
        }
    }
    if max_distance > tolerance {
        let left = simplify_points(&points[..=index], tolerance);
        let right = simplify_points(&points[index..], tolerance);
        let mut out = left[..left.len() - 1].to_vec();
        out.extend(right);
        out
    } else {
        vec![start, end]
    }
}

fn serialize_polyline(points: &[Point], decimals: usize) -> String {
    if points.is_empty() {
        return String::new();
    }
    let mut parts = vec![format!("M{} {}", fmt_number(points[0].x, decimals, false), fmt_number(points[0].y, decimals, false))];
    for point in &points[1..] {
        parts.push(format!("L{} {}", fmt_number(point.x, decimals, false), fmt_number(point.y, decimals, false)));
    }
    parts.join(" ")
}

fn serialize_smooth(points: &[Point], decimals: usize) -> String {
    if points.len() < 3 {
        return serialize_polyline(points, decimals);
    }
    let mut parts = vec![format!("M{} {}", fmt_number(points[0].x, decimals, false), fmt_number(points[0].y, decimals, false))];
    for i in 0..points.len() - 1 {
        let p0 = if i > 0 { points[i - 1] } else { points[i] };
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = if i + 2 < points.len() { points[i + 2] } else { p2 };
        let c1 = Point::new(p1.x + (p2.x - p0.x) / 6.0, p1.y + (p2.y - p0.y) / 6.0);
        let c2 = Point::new(p2.x - (p3.x - p1.x) / 6.0, p2.y - (p3.y - p1.y) / 6.0);
        parts.push(format!(
            "C{} {} {} {} {} {}",
            fmt_number(c1.x, decimals, false),
            fmt_number(c1.y, decimals, false),
            fmt_number(c2.x, decimals, false),
            fmt_number(c2.y, decimals, false),
            fmt_number(p2.x, decimals, false),
            fmt_number(p2.y, decimals, false)
        ));
    }
    parts.join(" ")
}

fn estimate_stroke_width(stroke_width: &str, skeleton: &HashSet<(usize, usize)>, distances: &HashMap<(usize, usize), f64>, scale: f64) -> Result<f64> {
    if stroke_width != "auto" {
        let width: f64 = stroke_width.parse().map_err(|_| SvgoError("--stroke-width must be a number or 'auto'".to_string()))?;
        if width <= 0.0 {
            return Err("--stroke-width must be greater than zero".into());
        }
        return Ok(width);
    }
    let mut values: Vec<f64> = skeleton
        .iter()
        .filter_map(|pixel| distances.get(pixel).copied())
        .filter(|d| *d > 0.0)
        .map(|d| d / scale)
        .collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    if values.is_empty() {
        return Err("Could not estimate stroke width from skeleton distances".into());
    }
    let lo = (values.len() as f64 * 0.15) as usize;
    let hi = (lo + 1).max((values.len() as f64 * 0.85) as usize).min(values.len());
    let core = &values[lo..hi];
    Ok(core[core.len() / 2] * 2.0)
}

fn centerline_path_data_core(path_data: &str, options: CenterlineOptionsCore) -> Result<(String, f64, RasterContext)> {
    if !["longest", "all"].contains(&options.mode.as_str()) {
        return Err("--mode must be longest or all".into());
    }
    if options.fill_rule != "evenodd" {
        return Err("Only evenodd fill rule is supported".into());
    }
    let subpaths = flatten_path(path_data, options.curve_samples)?;
    let ctx = make_context(&subpaths, options.scale, options.max_size)?;
    let mask = rasterize(&subpaths, ctx)?;
    let skeleton = zhang_suen_thin(&mask, ctx.height, ctx.width)?;
    let distances = chamfer_distance(&mask, ctx.height, ctx.width);
    let stroke_width = estimate_stroke_width(&options.stroke_width, &skeleton, &distances, ctx.scale)?;
    let skeleton = bridge_skeleton_gaps(&skeleton, ctx.height, ctx.width, options.bridge_gap.max(0.0) * ctx.scale);
    let min_length_px = (options.min_length * ctx.scale).max(0.0);
    let pixel_paths = trace_skeleton(&skeleton, &options.mode, min_length_px);
    let mut svg_paths = Vec::new();
    for pixel_path in pixel_paths {
        let mut points: Vec<_> = pixel_path.into_iter().map(|pixel| to_svg_point(pixel, ctx)).collect();
        points = simplify_points(&points, options.simplify.max(0.0));
        if points.len() < 2 {
            continue;
        }
        svg_paths.push(if options.polyline {
            serialize_polyline(&points, options.decimals)
        } else {
            serialize_smooth(&points, options.decimals)
        });
    }
    if svg_paths.is_empty() {
        return Err("No centerline paths survived simplification".into());
    }
    Ok((svg_paths.join(" "), stroke_width, ctx))
}

#[pyfunction]
fn centerline_path_data_json(path_data: &str, options_json: Option<&str>) -> PyResult<String> {
    let options = parse_center_options(options_json).map_err(py_err)?;
    let (d, stroke_width, ctx) = centerline_path_data_core(path_data, options).map_err(py_err)?;
    Ok(json!({"d": d, "stroke_width": stroke_width, "ctx": ctx}).to_string())
}

fn source_stroke_color(element: &Element, inherited: &HashMap<String, String>) -> String {
    let style = parse_style(attr_value(element, "style"));
    if let Some(stroke) = style
        .get("stroke")
        .map(String::as_str)
        .or_else(|| attr_value(element, "stroke"))
        .or_else(|| inherited.get("stroke").map(String::as_str))
    {
        if stroke != "none" {
            return stroke.to_string();
        }
    }
    if let Some(fill) = style
        .get("fill")
        .map(String::as_str)
        .or_else(|| attr_value(element, "fill"))
        .or_else(|| inherited.get("fill").map(String::as_str))
    {
        if fill != "none" {
            return fill.to_string();
        }
    }
    "currentColor".to_string()
}

fn source_opacity(element: &Element, inherited: &HashMap<String, String>) -> Option<String> {
    let style = parse_style(attr_value(element, "style"));
    for name in ["stroke-opacity", "fill-opacity", "opacity"] {
        if let Some(value) = style.get(name).cloned().or_else(|| attr_value(element, name).map(str::to_string)).or_else(|| inherited.get(name).cloned()) {
            return Some(value);
        }
    }
    None
}

fn inherited_graphics(element: &Element, inherited: &HashMap<String, String>) -> HashMap<String, String> {
    let mut current = inherited.clone();
    let style = parse_style(attr_value(element, "style"));
    for name in ["fill", "stroke", "opacity", "fill-opacity", "stroke-opacity"] {
        if let Some(value) = style.get(name).cloned().or_else(|| attr_value(element, name).map(str::to_string)) {
            current.insert(name.to_string(), value);
        }
    }
    current
}

fn svg_attrs(root: &Element) -> String {
    let mut attrs = vec![r#"xmlns="http://www.w3.org/2000/svg""#.to_string()];
    for name in ["viewBox", "width", "height"] {
        if let Some(value) = attr_value(root, name) {
            attrs.push(format!(r#"{}="{}""#, name, escape_attr(value)));
        }
    }
    attrs.join(" ")
}

fn centerline_svg_text_core(svg_text: &str, options: CenterlineOptionsCore) -> Result<String> {
    let root = parse_svg_element(svg_text).map_err(|err| SvgoError(format!("Could not parse SVG input: {}", err)))?;
    let mut output_paths = Vec::new();
    fn walk(element: &Element, inherited: &HashMap<String, String>, options: &CenterlineOptionsCore, output_paths: &mut Vec<String>) -> Result<()> {
        let current = inherited_graphics(element, inherited);
        if local_name(&element.name) == "path" {
            if let Some(d) = attr_value(element, "d") {
                match centerline_path_data_core(d, options.clone()) {
                    Ok((center_d, stroke_width, _ctx)) => {
                        let color = source_stroke_color(element, &current);
                        let opacity = source_opacity(element, &current);
                        let mut style = format!(
                            "fill: none; stroke: {}; stroke-linecap: {}; stroke-width: {}px; stroke-linejoin: {};",
                            color,
                            options.linecap,
                            fmt_number(stroke_width, options.decimals, false),
                            options.linejoin
                        );
                        if let Some(opacity) = opacity {
                            style.push_str(&format!(" stroke-opacity: {};", opacity));
                        }
                        output_paths.push(format!(r#"<path style="{}" d="{}"/>"#, escape_attr(&style), escape_attr(&center_d)));
                    }
                    Err(err) => {
                        if !options.keep_failed {
                            return Err(err);
                        }
                        output_paths.push(format!(r#"<path d="{}" fill="{}"/>"#, escape_attr(d), escape_attr(current.get("fill").map(String::as_str).unwrap_or("black"))));
                    }
                }
            }
        }
        for child in &element.children {
            if let XMLNode::Element(child_el) = child {
                walk(child_el, &current, options, output_paths)?;
            }
        }
        Ok(())
    }
    walk(&root, &HashMap::new(), &options, &mut output_paths)?;
    if output_paths.is_empty() {
        return Err("No path elements found in SVG input".into());
    }
    Ok(format!("<svg {}>\n  {}\n</svg>", svg_attrs(&root), output_paths.join("\n  ")))
}

#[pyfunction]
fn centerline_svg_text(svg_text: &str, options_json: Option<&str>) -> PyResult<String> {
    centerline_svg_text_core(svg_text, parse_center_options(options_json).map_err(py_err)?).map_err(py_err)
}

fn build_centerline_output(d: &str, emit: &str, stroke_width: f64, options: &CenterlineOptionsCore, ctx: RasterContext) -> String {
    if emit == "d" {
        return d.to_string();
    }
    let style = format!(
        "fill: none; stroke-linecap: {}; stroke-width: {}px; stroke-linejoin: {};",
        options.linecap,
        fmt_number(stroke_width, options.decimals, false),
        options.linejoin
    );
    let path = format!(r#"<path style="{}" d="{}"/>"#, style, escape_attr(d));
    if emit == "path" {
        return path;
    }
    let width = (ctx.width - ctx.pad * 2) as f64 / ctx.scale;
    let height = (ctx.height - ctx.pad * 2) as f64 / ctx.scale;
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{} {} {} {}">{}</svg>"#,
        fmt_number(ctx.min_x, options.decimals, false),
        fmt_number(ctx.min_y, options.decimals, false),
        fmt_number(width, options.decimals, false),
        fmt_number(height, options.decimals, false),
        path
    )
}

