#[derive(Debug, Clone, Serialize, Deserialize)]
struct Bounds {
    x: f64,
    y: f64,
    x2: f64,
    y2: f64,
}

impl Bounds {
    fn include_point(&mut self, point: Point) {
        self.x = self.x.min(point.x);
        self.y = self.y.min(point.y);
        self.x2 = self.x2.max(point.x);
        self.y2 = self.y2.max(point.y);
    }

    fn width(&self) -> f64 {
        self.x2 - self.x
    }

    fn height(&self) -> f64 {
        self.y2 - self.y
    }

    fn to_json(&self, decimals: Option<usize>) -> Value {
        json!({
            "x": round_json(self.x, decimals),
            "y": round_json(self.y, decimals),
            "x2": round_json(self.x2, decimals),
            "y2": round_json(self.y2, decimals),
            "width": round_json(self.width(), decimals),
            "height": round_json(self.height(), decimals),
            "cx": round_json((self.x + self.x2) / 2.0, decimals),
            "cy": round_json((self.y + self.y2) / 2.0, decimals),
        })
    }
}

#[derive(Debug, Clone)]
enum MeasureSegment {
    Line(Point, Point),
    Cubic(Point, Point, Point, Point),
}

fn measurable_segments(path_data: &str) -> Result<Vec<MeasureSegment>> {
    let parsed = PathDataCore::parse(path_data)?;
    let mut segments = Vec::new();
    for segment in parsed.segments {
        match (&segment.cmd, &segment.values) {
            ('M', _) => {}
            ('L', _) => append_line_segment(&mut segments, segment.start, segment.end),
            ('Q', SegmentValues::Quad(c)) => {
                let cubic = quadratic_to_cubic_segment(segment.start, *c, segment.end, segment.index);
                append_cubic_segment(&mut segments, &cubic)?;
            }
            ('C', _) => append_cubic_segment(&mut segments, &segment)?,
            ('A', _) => {
                for cubic in arc_to_cubic_segments(&segment)? {
                    if cubic.cmd == 'L' {
                        append_line_segment(&mut segments, cubic.start, cubic.end);
                    } else {
                        append_cubic_segment(&mut segments, &cubic)?;
                    }
                }
            }
            ('Z', _) => append_line_segment(&mut segments, segment.start, segment.end),
            _ => return Err(SvgoError(format!("Unsupported segment for measurement: {}", segment.cmd))),
        }
    }
    Ok(segments)
}

fn append_line_segment(segments: &mut Vec<MeasureSegment>, start: Point, end: Point) {
    if !start.close_to(end) {
        segments.push(MeasureSegment::Line(start, end));
    }
}

fn append_cubic_segment(segments: &mut Vec<MeasureSegment>, segment: &Segment) -> Result<()> {
    let SegmentValues::Cubic(c1, c2) = segment.values else {
        return Err("Cubic segment controls must be points".into());
    };
    if segment.start.close_to(segment.end) && segment.start.close_to(c1) && segment.end.close_to(c2) {
        return Ok(());
    }
    segments.push(MeasureSegment::Cubic(segment.start, c1, c2, segment.end));
    Ok(())
}

fn distance(a: Point, b: Point) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

fn lerp(a: Point, b: Point, t: f64) -> Point {
    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
}

fn cubic_point(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> Point {
    let mt = 1.0 - t;
    Point::new(
        mt.powi(3) * p0.x + 3.0 * mt.powi(2) * t * p1.x + 3.0 * mt * t.powi(2) * p2.x + t.powi(3) * p3.x,
        mt.powi(3) * p0.y + 3.0 * mt.powi(2) * t * p1.y + 3.0 * mt * t.powi(2) * p2.y + t.powi(3) * p3.y,
    )
}

fn split_cubic(p0: Point, p1: Point, p2: Point, p3: Point, t: f64) -> ((Point, Point, Point, Point), (Point, Point, Point, Point)) {
    let p01 = lerp(p0, p1, t);
    let p12 = lerp(p1, p2, t);
    let p23 = lerp(p2, p3, t);
    let p012 = lerp(p01, p12, t);
    let p123 = lerp(p12, p23, t);
    let p0123 = lerp(p012, p123, t);
    ((p0, p01, p012, p0123), (p0123, p123, p23, p3))
}

fn cubic_length(p0: Point, p1: Point, p2: Point, p3: Point, error: f64, depth: usize) -> f64 {
    let chord = distance(p0, p3);
    let control = distance(p0, p1) + distance(p1, p2) + distance(p2, p3);
    if depth >= 16 || control - chord <= error {
        return (control + chord) / 2.0;
    }
    let (left, right) = split_cubic(p0, p1, p2, p3, 0.5);
    cubic_length(left.0, left.1, left.2, left.3, error / 2.0, depth + 1)
        + cubic_length(right.0, right.1, right.2, right.3, error / 2.0, depth + 1)
}

fn segment_length(segment: &MeasureSegment, error: f64) -> f64 {
    match *segment {
        MeasureSegment::Line(a, b) => distance(a, b),
        MeasureSegment::Cubic(p0, p1, p2, p3) => cubic_length(p0, p1, p2, p3, error.max(1e-9), 0),
    }
}

fn point_on_segment_at_length(segment: &MeasureSegment, target: f64, error: f64) -> Point {
    match *segment {
        MeasureSegment::Line(a, b) => {
            let length = distance(a, b);
            if length == 0.0 {
                b
            } else {
                lerp(a, b, (target / length).clamp(0.0, 1.0))
            }
        }
        MeasureSegment::Cubic(p0, p1, p2, p3) => {
            let total = cubic_length(p0, p1, p2, p3, error, 0);
            if total == 0.0 {
                return p3;
            }
            let mut lo = 0.0;
            let mut hi = 1.0;
            for _ in 0..32 {
                let mid = (lo + hi) / 2.0;
                let left = split_cubic(p0, p1, p2, p3, mid).0;
                let length = cubic_length(left.0, left.1, left.2, left.3, error, 0);
                if length < target {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            cubic_point(p0, p1, p2, p3, (lo + hi) / 2.0)
        }
    }
}

fn cubic_derivative_roots(p0: f64, p1: f64, p2: f64, p3: f64) -> Vec<f64> {
    let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
    let b = 3.0 * p0 - 6.0 * p1 + 3.0 * p2;
    let c = -3.0 * p0 + 3.0 * p1;
    let qa = 3.0 * a;
    let qb = 2.0 * b;
    let qc = c;
    if qa.abs() < 1e-12 {
        if qb.abs() < 1e-12 {
            return vec![];
        }
        return vec![-qc / qb];
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return vec![];
    }
    let root = disc.max(0.0).sqrt();
    vec![(-qb - root) / (2.0 * qa), (-qb + root) / (2.0 * qa)]
}

fn cubic_extrema(p0: Point, p1: Point, p2: Point, p3: Point) -> Vec<f64> {
    let mut roots = vec![0.0, 1.0];
    for root in cubic_derivative_roots(p0.x, p1.x, p2.x, p3.x)
        .into_iter()
        .chain(cubic_derivative_roots(p0.y, p1.y, p2.y, p3.y))
    {
        if root > 0.0 && root < 1.0 {
            roots.push(root);
        }
    }
    roots.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    roots.dedup_by(|a, b| (*a - *b).abs() < 1e-12);
    roots
}

fn segments_bounds(segments: &[MeasureSegment]) -> Option<Bounds> {
    let mut bounds: Option<Bounds> = None;
    let mut include = |point: Point| {
        if let Some(b) = &mut bounds {
            b.include_point(point);
        } else {
            bounds = Some(Bounds {
                x: point.x,
                y: point.y,
                x2: point.x,
                y2: point.y,
            });
        }
    };
    for segment in segments {
        match *segment {
            MeasureSegment::Line(a, b) => {
                include(a);
                include(b);
            }
            MeasureSegment::Cubic(p0, p1, p2, p3) => {
                for t in cubic_extrema(p0, p1, p2, p3) {
                    include(cubic_point(p0, p1, p2, p3, t));
                }
            }
        }
    }
    bounds
}

fn round_json(value: f64, decimals: Option<usize>) -> Value {
    match decimals {
        Some(d) => json!(round_to(value, d)),
        None => json!(value),
    }
}

fn path_metrics_value(path_data: &str, decimals: Option<usize>, error: f64) -> Result<Value> {
    let segments = measurable_segments(path_data)?;
    let bounds = segments_bounds(&segments);
    let length: f64 = segments.iter().map(|s| segment_length(s, error)).sum();
    Ok(json!({
        "length": round_json(length, decimals),
        "bbox": bounds.map(|b| b.to_json(decimals)).unwrap_or(Value::Null),
        "segments": segments.len(),
    }))
}

#[pyfunction]
fn path_length(path_data: &str, error: Option<f64>) -> PyResult<f64> {
    let segments = measurable_segments(path_data).map_err(py_err)?;
    Ok(segments.iter().map(|s| segment_length(s, error.unwrap_or(0.01))).sum())
}

#[pyfunction]
fn path_bbox_json(path_data: &str, decimals: Option<usize>) -> PyResult<String> {
    let segments = measurable_segments(path_data).map_err(py_err)?;
    Ok(segments_bounds(&segments)
        .map(|b| b.to_json(decimals))
        .unwrap_or(Value::Null)
        .to_string())
}

#[pyfunction]
fn path_metrics_json(path_data: &str, decimals: Option<usize>, error: Option<f64>) -> PyResult<String> {
    Ok(path_metrics_value(path_data, decimals, error.unwrap_or(0.01))
        .map_err(py_err)?
        .to_string())
}

#[pyfunction]
fn point_at_length_json(path_data: &str, distance_value: f64, error: Option<f64>) -> PyResult<String> {
    let segments = measurable_segments(path_data).map_err(py_err)?;
    if segments.is_empty() {
        return Err(PyValueError::new_err("Path contains no drawable segments"));
    }
    if distance_value <= 0.0 {
        let first = match segments[0] {
            MeasureSegment::Line(a, _) | MeasureSegment::Cubic(a, _, _, _) => a,
        };
        return Ok(json!({"x": first.x, "y": first.y}).to_string());
    }
    let mut remaining = distance_value;
    let mut last = match segments.last().unwrap() {
        MeasureSegment::Line(_, b) | MeasureSegment::Cubic(_, _, _, b) => *b,
    };
    let err = error.unwrap_or(0.01);
    for segment in &segments {
        let length = segment_length(segment, err);
        if remaining <= length {
            let point = point_on_segment_at_length(segment, remaining, (err / 10.0).max(1e-6));
            return Ok(json!({"x": point.x, "y": point.y}).to_string());
        }
        remaining -= length;
        last = match segment {
            MeasureSegment::Line(_, b) | MeasureSegment::Cubic(_, _, _, b) => *b,
        };
    }
    Ok(json!({"x": last.x, "y": last.y}).to_string())
}

