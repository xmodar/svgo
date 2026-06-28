#[derive(Debug, Clone)]
struct PathDataCore {
    segments: Vec<Segment>,
    relative: bool,
    optimize_flags: HashMap<String, bool>,
}

impl PathDataCore {
    fn parse(path_data: &str) -> Result<Self> {
        let tokens = tokenize_path(path_data);
        if tokens.is_empty() {
            return Err("No SVG path tokens found".into());
        }
        let mut stream = TokenStream::new(tokens);
        let mut segments = Vec::new();
        let mut command = String::new();
        let mut current = Point::new(0.0, 0.0);
        let mut subpath_start = Point::new(0.0, 0.0);
        let mut last_cubic_ctrl: Option<Point> = None;
        let mut last_quad_ctrl: Option<Point> = None;
        let mut index = 0usize;

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
                Ok(if relative {
                    Point::new(current.x + x, current.y + y)
                } else {
                    Point::new(x, y)
                })
            };

            match upper.as_str() {
                "M" => {
                    if !stream.has_numbers(2) {
                        return Err("M command requires x y".into());
                    }
                    current = read_point(&mut stream, current)?;
                    subpath_start = current;
                    segments.push(Segment::new('M', current, current, SegmentValues::None, index));
                    index += 1;
                    while stream.has_numbers(2) {
                        let start = current;
                        current = read_point(&mut stream, current)?;
                        segments.push(Segment::new('L', start, current, SegmentValues::None, index));
                        index += 1;
                    }
                    command = if relative { "l" } else { "L" }.to_string();
                    last_cubic_ctrl = None;
                    last_quad_ctrl = None;
                }
                "L" => {
                    while stream.has_numbers(2) {
                        let start = current;
                        current = read_point(&mut stream, current)?;
                        segments.push(Segment::new('L', start, current, SegmentValues::None, index));
                        index += 1;
                    }
                    last_cubic_ctrl = None;
                    last_quad_ctrl = None;
                }
                "H" => {
                    while stream.has_numbers(1) {
                        let start = current;
                        let x = stream.number("x")?;
                        current = if relative {
                            Point::new(current.x + x, current.y)
                        } else {
                            Point::new(x, current.y)
                        };
                        segments.push(Segment::new('L', start, current, SegmentValues::None, index));
                        index += 1;
                    }
                    last_cubic_ctrl = None;
                    last_quad_ctrl = None;
                }
                "V" => {
                    while stream.has_numbers(1) {
                        let start = current;
                        let y = stream.number("y")?;
                        current = if relative {
                            Point::new(current.x, current.y + y)
                        } else {
                            Point::new(current.x, y)
                        };
                        segments.push(Segment::new('L', start, current, SegmentValues::None, index));
                        index += 1;
                    }
                    last_cubic_ctrl = None;
                    last_quad_ctrl = None;
                }
                "C" => {
                    while stream.has_numbers(6) {
                        let start = current;
                        let c1 = read_point(&mut stream, current)?;
                        let c2 = read_point(&mut stream, current)?;
                        current = read_point(&mut stream, current)?;
                        segments.push(Segment::new(
                            'C',
                            start,
                            current,
                            SegmentValues::Cubic(c1, c2),
                            index,
                        ));
                        index += 1;
                        last_cubic_ctrl = Some(c2);
                        last_quad_ctrl = None;
                    }
                }
                "S" => {
                    while stream.has_numbers(4) {
                        let start = current;
                        let c1 = last_cubic_ctrl
                            .map(|c| Point::new(2.0 * current.x - c.x, 2.0 * current.y - c.y))
                            .unwrap_or(current);
                        let c2 = read_point(&mut stream, current)?;
                        current = read_point(&mut stream, current)?;
                        segments.push(Segment::new(
                            'C',
                            start,
                            current,
                            SegmentValues::Cubic(c1, c2),
                            index,
                        ));
                        index += 1;
                        last_cubic_ctrl = Some(c2);
                        last_quad_ctrl = None;
                    }
                }
                "Q" => {
                    while stream.has_numbers(4) {
                        let start = current;
                        let c = read_point(&mut stream, current)?;
                        current = read_point(&mut stream, current)?;
                        segments.push(Segment::new('Q', start, current, SegmentValues::Quad(c), index));
                        index += 1;
                        last_quad_ctrl = Some(c);
                        last_cubic_ctrl = None;
                    }
                }
                "T" => {
                    while stream.has_numbers(2) {
                        let start = current;
                        let c = last_quad_ctrl
                            .map(|q| Point::new(2.0 * current.x - q.x, 2.0 * current.y - q.y))
                            .unwrap_or(current);
                        current = read_point(&mut stream, current)?;
                        segments.push(Segment::new('Q', start, current, SegmentValues::Quad(c), index));
                        index += 1;
                        last_quad_ctrl = Some(c);
                        last_cubic_ctrl = None;
                    }
                }
                "A" => {
                    while stream.has_more() && stream.peek().is_some_and(|t| !is_command_token(t)) {
                        let start = current;
                        let rx = stream.number("rx")?;
                        let ry = stream.number("ry")?;
                        let rotation = stream.number("x-axis-rotation")?;
                        let large_arc = stream.flag("large-arc-flag")?;
                        let sweep = stream.flag("sweep-flag")?;
                        current = read_point(&mut stream, current)?;
                        segments.push(Segment::new(
                            'A',
                            start,
                            current,
                            SegmentValues::Arc {
                                rx: rx.abs(),
                                ry: ry.abs(),
                                rotation,
                                large_arc,
                                sweep,
                            },
                            index,
                        ));
                        index += 1;
                    }
                    last_cubic_ctrl = None;
                    last_quad_ctrl = None;
                }
                "Z" => {
                    segments.push(Segment::new(
                        'Z',
                        current,
                        subpath_start,
                        SegmentValues::None,
                        index,
                    ));
                    index += 1;
                    current = subpath_start;
                    last_cubic_ctrl = None;
                    last_quad_ctrl = None;
                }
                _ => return Err(SvgoError(format!("Unsupported path command: {}", command))),
            }
        }

        if segments.is_empty() {
            return Err("Path did not produce any commands".into());
        }
        Ok(Self {
            segments: reindex_segments(recompute_starts(&segments)),
            relative: false,
            optimize_flags: HashMap::new(),
        })
    }

    fn transform(&mut self, matrix: Matrix) -> Result<&mut Self> {
        let mut transformed = Vec::new();
        for segment in &self.segments {
            transformed.extend(segment.transformed(matrix)?);
        }
        self.segments = reindex_segments(recompute_starts(&transformed));
        Ok(self)
    }

    fn translate(&mut self, dx: f64, dy: f64) -> Result<&mut Self> {
        self.transform(translate_matrix(dx, dy))
    }

    fn scale(&mut self, kx: f64, ky: f64) -> Result<&mut Self> {
        self.transform(scale_matrix(kx, ky))
    }

    fn rotate(&mut self, ox: f64, oy: f64, degrees: f64) -> Result<&mut Self> {
        self.transform(rotate_matrix(ox, oy, degrees))
    }

    fn set_relative(&mut self, relative: bool) -> &mut Self {
        self.relative = relative;
        self
    }

    fn reverse(&mut self, item_index: Option<usize>) -> Result<&mut Self> {
        let groups = subpath_groups(&self.segments);
        let selected: HashSet<usize> = if let Some(index) = item_index {
            [group_index_for_item(&groups, index)?].into_iter().collect()
        } else {
            (0..groups.len()).collect()
        };
        let mut result = Vec::new();
        for (i, group) in groups.iter().enumerate() {
            if selected.contains(&i) {
                result.extend(reverse_group(group)?);
            } else {
                result.extend(group.clone());
            }
        }
        self.segments = reindex_segments(recompute_starts(&result));
        Ok(self)
    }

    fn change_origin(&mut self, item_index: usize, subpath: bool) -> Result<&mut Self> {
        let mut groups = subpath_groups(&self.segments);
        if subpath {
            if item_index >= groups.len() {
                return Err(SvgoError(format!(
                    "origin subpath index {} is out of range",
                    item_index
                )));
            }
            let mut reordered = Vec::new();
            for group in groups[item_index..].iter().chain(groups[..item_index].iter()) {
                reordered.extend(group.clone());
            }
            self.segments = reindex_segments(reordered);
            return Ok(self);
        }
        let group_index = group_index_for_item(&groups, item_index)?;
        groups[group_index] = rotate_group_origin(&groups[group_index], item_index)?;
        let flattened: Vec<_> = groups.into_iter().flatten().collect();
        self.segments = reindex_segments(recompute_starts(&flattened));
        Ok(self)
    }

    fn optimize(&mut self, profile: Option<&str>) -> Result<&mut Self> {
        let options = parse_optimize_options(profile.unwrap_or("safe"))?;
        if options.get("removeUselessCommands").copied().unwrap_or(false)
            || options.get("removeOrphanDots").copied().unwrap_or(false)
        {
            self.segments = remove_useless(
                &self.segments,
                options.get("removeOrphanDots").copied().unwrap_or(false),
            );
        }
        if options.get("useClosePath").copied().unwrap_or(false) {
            self.segments = close_matching_subpaths(&self.segments);
        }
        if options.get("useReverse").copied().unwrap_or(false) {
            self.segments = choose_shorter_reversal(&self.segments)?;
        }
        self.optimize_flags.extend(options);
        self.segments = reindex_segments(recompute_starts(&self.segments));
        Ok(self)
    }

    fn to_cubics(&mut self) -> Result<&mut Self> {
        self.segments = path_segments_to_cubics(&self.segments)?;
        self.relative = false;
        Ok(self)
    }

    fn apply_operation(&mut self, operation: &str, _decimals: usize) -> Result<&mut Self> {
        let (name, rest) = split_operation(operation);
        match name.as_str() {
            "translate" => {
                let nums = parse_number_list(&rest, 2, operation)?;
                self.translate(nums[0], nums[1])
            }
            "scale" => {
                let nums = parse_number_list(&rest, 2, operation)?;
                self.scale(nums[0], nums[1])
            }
            "matrix" => self.transform(parse_matrix_values(&rest)?),
            "rotate" => {
                let nums = parse_number_list(&rest, 3, operation)?;
                self.rotate(nums[0], nums[1], nums[2])
            }
            "relative" => Ok(self.set_relative(true)),
            "absolute" => Ok(self.set_relative(false)),
            "reverse" => {
                let item = if rest.is_empty() {
                    None
                } else {
                    Some(parse_non_negative_int(&rest, "reverse itemIndex")?)
                };
                self.reverse(item)
            }
            "origin" => {
                let parts: Vec<_> = rest.split(':').collect();
                if parts.is_empty() || parts[0].is_empty() {
                    return Err("origin requires itemIndex".into());
                }
                self.change_origin(
                    parse_non_negative_int(parts[0], "origin itemIndex")?,
                    parts.get(1).is_some_and(|v| *v == "subpath"),
                )
            }
            "optimize" => self.optimize(Some(if rest.is_empty() { "safe" } else { &rest })),
            "cubics" | "cubic" | "to-cubics" | "toCubics" => self.to_cubics(),
            _ => Err(SvgoError(format!("Unknown operation: {}", name))),
        }
    }

    fn to_string(&self, decimals: usize, minify: bool) -> String {
        serialize_path(
            &self.segments,
            decimals,
            minify,
            self.relative,
            Some(&self.optimize_flags),
        )
    }

    fn command_items(&self) -> Value {
        let mut items = Vec::new();
        for segment in &self.segments {
            let (command, args) = match (&segment.cmd, &segment.values) {
                ('M', _) => ("M", vec![segment.end.x, segment.end.y]),
                ('L', _) => ("L", vec![segment.end.x, segment.end.y]),
                ('C', SegmentValues::Cubic(c1, c2)) => (
                    "C",
                    vec![c1.x, c1.y, c2.x, c2.y, segment.end.x, segment.end.y],
                ),
                ('Q', SegmentValues::Quad(c)) => ("Q", vec![c.x, c.y, segment.end.x, segment.end.y]),
                (
                    'A',
                    SegmentValues::Arc {
                        rx,
                        ry,
                        rotation,
                        large_arc,
                        sweep,
                    },
                ) => (
                    "A",
                    vec![
                        *rx,
                        *ry,
                        *rotation,
                        *large_arc as f64,
                        *sweep as f64,
                        segment.end.x,
                        segment.end.y,
                    ],
                ),
                ('Z', _) => ("Z", vec![]),
                _ => ("?", vec![]),
            };
            items.push(json!({"command": command, "args": args, "index": segment.index}));
        }
        Value::Array(items)
    }
}

#[pyclass(name = "PathData", skip_from_py_object)]
#[derive(Clone)]
struct PyPathData {
    inner: PathDataCore,
}

#[pymethods]
impl PyPathData {
    #[staticmethod]
    fn parse(path_data: &str) -> PyResult<Self> {
        Ok(Self {
            inner: PathDataCore::parse(path_data).map_err(py_err)?,
        })
    }

    fn apply_operation(&mut self, operation: &str, decimals: Option<usize>) -> PyResult<()> {
        self.inner
            .apply_operation(operation, decimals.unwrap_or(4))
            .map_err(py_err)?;
        Ok(())
    }

    fn transform(&mut self, matrix: Vec<f64>) -> PyResult<()> {
        self.inner.transform(coerce_matrix_slice(&matrix).map_err(py_err)?).map_err(py_err)?;
        Ok(())
    }

    fn translate(&mut self, dx: f64, dy: f64) -> PyResult<()> {
        self.inner.translate(dx, dy).map_err(py_err)?;
        Ok(())
    }

    fn scale(&mut self, kx: f64, ky: f64) -> PyResult<()> {
        self.inner.scale(kx, ky).map_err(py_err)?;
        Ok(())
    }

    fn rotate(&mut self, ox: f64, oy: f64, degrees: f64) -> PyResult<()> {
        self.inner.rotate(ox, oy, degrees).map_err(py_err)?;
        Ok(())
    }

    fn set_relative(&mut self, relative: bool) {
        self.inner.set_relative(relative);
    }

    fn reverse(&mut self, item_index: Option<usize>) -> PyResult<()> {
        self.inner.reverse(item_index).map_err(py_err)?;
        Ok(())
    }

    fn change_origin(&mut self, item_index: usize, subpath: Option<bool>) -> PyResult<()> {
        self.inner
            .change_origin(item_index, subpath.unwrap_or(false))
            .map_err(py_err)?;
        Ok(())
    }

    fn optimize(&mut self, profile: Option<&str>) -> PyResult<()> {
        self.inner.optimize(profile).map_err(py_err)?;
        Ok(())
    }

    fn to_cubics(&mut self) -> PyResult<()> {
        self.inner.to_cubics().map_err(py_err)?;
        Ok(())
    }

    #[pyo3(signature = (decimals=4, minify=false))]
    fn to_string(&self, decimals: usize, minify: bool) -> String {
        self.inner.to_string(decimals, minify)
    }

    fn command_items_json(&self) -> String {
        self.inner.command_items().to_string()
    }
}

struct TokenStream {
    tokens: Vec<String>,
    i: usize,
}

impl TokenStream {
    fn new(tokens: Vec<String>) -> Self {
        Self { tokens, i: 0 }
    }

    fn has_more(&self) -> bool {
        self.i < self.tokens.len()
    }

    fn peek(&self) -> Option<&str> {
        self.tokens.get(self.i).map(String::as_str)
    }

    fn next(&mut self) -> Result<String> {
        if self.i >= self.tokens.len() {
            return Err("Unexpected end of path data".into());
        }
        let token = self.tokens[self.i].clone();
        self.i += 1;
        Ok(token)
    }

    fn has_numbers(&self, count: usize) -> bool {
        self.i + count <= self.tokens.len()
            && (0..count).all(|j| !is_command_token(&self.tokens[self.i + j]))
    }

    fn number(&mut self, label: &str) -> Result<f64> {
        let token = self.next()?;
        if is_command_token(&token) {
            return Err(SvgoError(format!("{} requires a number", label)));
        }
        token
            .parse::<f64>()
            .map_err(|_| SvgoError(format!("Invalid numeric token: {}", token)))
    }

    fn flag(&mut self, label: &str) -> Result<i32> {
        if self.i >= self.tokens.len() || is_command_token(&self.tokens[self.i]) {
            return Err(SvgoError(format!("Arc command requires {}", label)));
        }
        let token = self.tokens[self.i].clone();
        if token == "0" || token == "1" {
            self.i += 1;
            return Ok(token.parse().unwrap());
        }
        if token.starts_with('0') || token.starts_with('1') {
            let flag = token[..1].parse::<i32>().unwrap();
            let rest = token[1..].to_string();
            if rest.is_empty() {
                self.i += 1;
            } else {
                self.tokens[self.i] = rest;
            }
            return Ok(flag);
        }
        Err(SvgoError(format!(
            "Arc command {} must be 0 or 1",
            label
        )))
    }
}

fn tokenize_path(path_data: &str) -> Vec<String> {
    let chars: Vec<char> = path_data.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_whitespace() || c == ',' {
            i += 1;
            continue;
        }
        if c.is_ascii_alphabetic() {
            out.push(c.to_string());
            i += 1;
            continue;
        }
        if c == '+' || c == '-' || c == '.' || c.is_ascii_digit() {
            let start = i;
            if c == '+' || c == '-' {
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
                let exp = i;
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
                let before_digits = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                if before_digits == i {
                    i = exp;
                }
            }
            out.push(chars[start..i].iter().collect());
            continue;
        }
        i += 1;
    }
    out
}

fn is_command_token(token: &str) -> bool {
    token.len() == 1 && token.as_bytes()[0].is_ascii_alphabetic()
}

fn matrix_multiply(left: Matrix, right: Matrix) -> Matrix {
    let [a1, b1, c1, d1, e1, f1] = left;
    let [a2, b2, c2, d2, e2, f2] = right;
    [
        a1 * a2 + c1 * b2,
        b1 * a2 + d1 * b2,
        a1 * c2 + c1 * d2,
        b1 * c2 + d1 * d2,
        a1 * e2 + c1 * f2 + e1,
        b1 * e2 + d1 * f2 + f1,
    ]
}

fn translate_matrix(dx: f64, dy: f64) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, dx, dy]
}

fn scale_matrix(kx: f64, ky: f64) -> Matrix {
    [kx, 0.0, 0.0, ky, 0.0, 0.0]
}

fn rotate_matrix(ox: f64, oy: f64, degrees: f64) -> Matrix {
    let radians = degrees.to_radians();
    let cos_v = radians.cos();
    let sin_v = radians.sin();
    let around_origin = [cos_v, sin_v, -sin_v, cos_v, 0.0, 0.0];
    matrix_multiply(
        translate_matrix(ox, oy),
        matrix_multiply(around_origin, translate_matrix(-ox, -oy)),
    )
}

fn parse_matrix_values(text: &str) -> Result<Matrix> {
    let parts: Vec<_> = text
        .split(|c: char| c == ',' || c.is_ascii_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 6 {
        return Err("matrix requires 6 comma- or space-separated numbers".into());
    }
    let mut out = [0.0; 6];
    for (i, part) in parts.iter().enumerate() {
        out[i] = part
            .parse()
            .map_err(|_| SvgoError(format!("Invalid matrix value in {:?}", text)))?;
    }
    Ok(out)
}

fn parse_transform(transform: &str) -> Result<Matrix> {
    let mut matrix = IDENTITY;
    let mut i = 0usize;
    let bytes = transform.as_bytes();
    while i < bytes.len() {
        while i < bytes.len() && (bytes[i].is_ascii_whitespace() || bytes[i] == b',') {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if name_start == i {
            break;
        }
        let name = &transform[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'(' {
            return Err(SvgoError(format!("Unsupported transform: {}", name)));
        }
        i += 1;
        let raw_start = i;
        while i < bytes.len() && bytes[i] != b')' {
            i += 1;
        }
        let raw = &transform[raw_start..i.min(bytes.len())];
        if i < bytes.len() {
            i += 1;
        }
        let values = parse_float_list(raw);
        let op = match name {
            "matrix" => {
                if values.len() != 6 {
                    return Err("matrix() transform requires 6 values".into());
                }
                [values[0], values[1], values[2], values[3], values[4], values[5]]
            }
            "translate" => {
                if values.is_empty() {
                    return Err("translate() transform requires at least 1 value".into());
                }
                translate_matrix(values[0], *values.get(1).unwrap_or(&0.0))
            }
            "scale" => {
                if values.is_empty() {
                    return Err("scale() transform requires at least 1 value".into());
                }
                scale_matrix(values[0], *values.get(1).unwrap_or(&values[0]))
            }
            "rotate" => match values.len() {
                1 => rotate_matrix(0.0, 0.0, values[0]),
                3 => rotate_matrix(values[1], values[2], values[0]),
                _ => return Err("rotate() transform requires 1 or 3 values".into()),
            },
            "skewX" => {
                if values.len() != 1 {
                    return Err("skewX() transform requires 1 value".into());
                }
                [1.0, 0.0, values[0].to_radians().tan(), 1.0, 0.0, 0.0]
            }
            "skewY" => {
                if values.len() != 1 {
                    return Err("skewY() transform requires 1 value".into());
                }
                [1.0, values[0].to_radians().tan(), 0.0, 1.0, 0.0, 0.0]
            }
            _ => return Err(SvgoError(format!("Unsupported transform: {}", name))),
        };
        matrix = matrix_multiply(matrix, op);
    }
    Ok(matrix)
}

fn coerce_matrix_slice(values: &[f64]) -> Result<Matrix> {
    if values.len() == 6 {
        Ok([values[0], values[1], values[2], values[3], values[4], values[5]])
    } else if values.len() == 9 {
        Ok([values[0], values[3], values[1], values[4], values[2], values[5]])
    } else {
        Err("matrix must contain 6 SVG affine values or 9 row-major 3x3 values".into())
    }
}

fn parse_float_list(text: &str) -> Vec<f64> {
    tokenize_path(text)
        .into_iter()
        .filter(|t| !is_command_token(t))
        .filter_map(|t| t.parse::<f64>().ok())
        .collect()
}

fn parse_optimize_options(profile: &str) -> Result<HashMap<String, bool>> {
    let mut options = HashMap::new();
    match profile {
        "" | "safe" => {
            options.insert("removeUselessCommands".to_string(), true);
            options.insert("useShorthands".to_string(), true);
            options.insert("useHorizontalAndVerticalLines".to_string(), true);
            options.insert("useRelativeAbsolute".to_string(), true);
        }
        "size" => {
            options = parse_optimize_options("safe")?;
            options.insert("useReverse".to_string(), true);
        }
        "closed" => {
            options = parse_optimize_options("safe")?;
            options.insert("useClosePath".to_string(), true);
        }
        "all" => {
            for key in [
                "removeUselessCommands",
                "removeOrphanDots",
                "useShorthands",
                "useHorizontalAndVerticalLines",
                "useRelativeAbsolute",
                "useReverse",
                "useClosePath",
            ] {
                options.insert(key.to_string(), true);
            }
        }
        other => {
            for raw in other.split(',') {
                let name = raw.trim();
                if name.is_empty() {
                    continue;
                }
                let key = match name {
                    "remove-useless" | "remove-useless-commands" | "removeUselessCommands" => {
                        "removeUselessCommands"
                    }
                    "use-shorthands" | "useShorthands" => "useShorthands",
                    "use-hv"
                    | "use-horizontal-vertical"
                    | "use-horizontal-and-vertical-lines"
                    | "useHorizontalAndVerticalLines" => "useHorizontalAndVerticalLines",
                    "use-relative-absolute" | "useRelativeAbsolute" => "useRelativeAbsolute",
                    "use-reverse" | "useReverse" => "useReverse",
                    "use-close-path" | "useClosePath" => "useClosePath",
                    "remove-orphan-dots" | "removeOrphanDots" => "removeOrphanDots",
                    _ => return Err(SvgoError(format!("Unknown optimize option: {}", name))),
                };
                options.insert(key.to_string(), true);
            }
        }
    }
    Ok(options)
}

fn reindex_segments(segments: Vec<Segment>) -> Vec<Segment> {
    segments
        .into_iter()
        .enumerate()
        .map(|(i, mut segment)| {
            segment.index = i;
            segment
        })
        .collect()
}

fn recompute_starts(segments: &[Segment]) -> Vec<Segment> {
    let mut result = Vec::new();
    let mut current = Point::new(0.0, 0.0);
    let mut subpath_start = Point::new(0.0, 0.0);
    for segment in segments {
        let mut seg = segment.clone();
        if seg.cmd == 'M' {
            current = seg.end;
            subpath_start = seg.end;
            seg.start = current;
            seg.end = current;
            result.push(seg);
        } else if seg.cmd == 'Z' {
            seg.start = current;
            seg.end = subpath_start;
            result.push(seg);
            current = subpath_start;
        } else {
            seg.start = current;
            current = seg.end;
            result.push(seg);
        }
    }
    result
}

fn subpath_groups(segments: &[Segment]) -> Vec<Vec<Segment>> {
    let mut groups: Vec<Vec<Segment>> = Vec::new();
    let mut current: Vec<Segment> = Vec::new();
    for segment in segments {
        if segment.cmd == 'M' && !current.is_empty() {
            groups.push(current);
            current = vec![segment.clone()];
        } else {
            current.push(segment.clone());
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    groups
}

fn group_index_for_item(groups: &[Vec<Segment>], item_index: usize) -> Result<usize> {
    for (group_index, group) in groups.iter().enumerate() {
        if group.iter().any(|segment| segment.index == item_index) {
            return Ok(group_index);
        }
    }
    Err(SvgoError(format!("item index {} is out of range", item_index)))
}

fn reverse_group(group: &[Segment]) -> Result<Vec<Segment>> {
    if group.is_empty() || group[0].cmd != 'M' {
        return Ok(group.to_vec());
    }
    let mut drawing: Vec<_> = group.iter().skip(1).filter(|s| s.cmd != 'Z').cloned().collect();
    if drawing.is_empty() {
        return Ok(group.to_vec());
    }
    let closed = group.iter().any(|s| s.cmd == 'Z');
    let start = group[0].end;
    if closed && !drawing.last().unwrap().end.close_to(start) {
        let last = drawing.last().unwrap().clone();
        drawing.push(Segment::new('L', last.end, start, SegmentValues::None, last.index));
    }
    let new_start = if closed { start } else { drawing.last().unwrap().end };
    let mut result = vec![Segment::new('M', new_start, new_start, SegmentValues::None, group[0].index)];
    for segment in drawing.iter().rev() {
        result.push(segment.reversed()?);
    }
    if closed {
        let last_end = result.last().unwrap().end;
        result.push(Segment::new(
            'Z',
            last_end,
            new_start,
            SegmentValues::None,
            group.last().unwrap().index,
        ));
    }
    Ok(recompute_starts(&result))
}

fn rotate_group_origin(group: &[Segment], item_index: usize) -> Result<Vec<Segment>> {
    if group.is_empty() || group[0].cmd != 'M' {
        return Ok(group.to_vec());
    }
    let drawing: Vec<_> = group.iter().skip(1).filter(|s| s.cmd != 'Z').cloned().collect();
    let closed = group.iter().any(|s| s.cmd == 'Z');
    if !closed {
        return Err("origin can only rotate closed subpaths".into());
    }
    if drawing.is_empty() {
        return Ok(group.to_vec());
    }
    let target = drawing
        .iter()
        .position(|segment| segment.index == item_index)
        .ok_or_else(|| {
            SvgoError(format!(
                "origin item index {} is not a drawable item in its subpath",
                item_index
            ))
        })?;
    let start = drawing[target].start;
    let mut rotated = drawing[target..].to_vec();
    rotated.extend_from_slice(&drawing[..target]);
    let mut result = vec![Segment::new('M', start, start, SegmentValues::None, group[0].index)];
    result.extend(rotated.iter().cloned());
    result.push(Segment::new(
        'Z',
        rotated.last().unwrap().end,
        start,
        SegmentValues::None,
        group.last().unwrap().index,
    ));
    Ok(recompute_starts(&result))
}

fn remove_useless(segments: &[Segment], remove_orphan_dots: bool) -> Vec<Segment> {
    let mut result: Vec<Segment> = Vec::new();
    let mut group_has_draw = false;
    for segment in segments {
        match segment.cmd {
            'M' => {
                if remove_orphan_dots
                    && result.last().is_some_and(|s| s.cmd == 'M')
                    && !group_has_draw
                {
                    result.pop();
                }
                result.push(segment.clone());
                group_has_draw = false;
            }
            'Z' => {
                if group_has_draw {
                    result.push(segment.clone());
                }
            }
            'L' | 'Q' | 'C' if segment.start.close_to(segment.end) => {}
            _ => {
                result.push(segment.clone());
                group_has_draw = true;
            }
        }
    }
    if remove_orphan_dots
        && result.last().is_some_and(|s| s.cmd == 'M')
        && !group_has_draw
    {
        result.pop();
    }
    result
}

fn close_matching_subpaths(segments: &[Segment]) -> Vec<Segment> {
    let mut result = Vec::new();
    for group in subpath_groups(segments) {
        if group.is_empty() || group[0].cmd != 'M' || group.iter().any(|s| s.cmd == 'Z') {
            result.extend(group);
            continue;
        }
        let drawing = &group[1..];
        if !drawing.is_empty() && drawing.last().unwrap().end.close_to(group[0].end) {
            result.extend_from_slice(&group[..group.len() - 1]);
            let z_start = if drawing.len() > 1 {
                drawing[drawing.len() - 2].end
            } else {
                group[0].end
            };
            result.push(Segment::new(
                'Z',
                z_start,
                group[0].end,
                SegmentValues::None,
                drawing.last().unwrap().index,
            ));
        } else {
            result.extend(group);
        }
    }
    result
}

fn choose_shorter_reversal(segments: &[Segment]) -> Result<Vec<Segment>> {
    let mut result = Vec::new();
    let flags = HashMap::from([
        ("useRelativeAbsolute".to_string(), true),
        ("useHorizontalAndVerticalLines".to_string(), true),
        ("useShorthands".to_string(), true),
    ]);
    for group in subpath_groups(segments) {
        let normal = serialize_path(&group, 4, true, false, Some(&flags));
        let reversed = reverse_group(&group)?;
        let reversed_text = serialize_path(&reversed, 4, true, false, Some(&flags));
        result.extend(if reversed_text.len() < normal.len() { reversed } else { group });
    }
    Ok(result)
}

fn serialize_path(
    segments: &[Segment],
    decimals: usize,
    minify: bool,
    relative_mode: bool,
    optimize_flags: Option<&HashMap<String, bool>>,
) -> String {
    let empty = HashMap::new();
    let flags = optimize_flags.unwrap_or(&empty);
    let mut parts = Vec::new();
    let mut current = Point::new(0.0, 0.0);
    let mut subpath_start = Point::new(0.0, 0.0);
    let mut last_cubic_ctrl: Option<Point> = None;
    let mut last_quad_ctrl: Option<Point> = None;

    for segment in recompute_starts(segments) {
        let absolute = segment_to_text(
            &segment,
            current,
            subpath_start,
            decimals,
            minify,
            false,
            flags,
            last_cubic_ctrl,
            last_quad_ctrl,
        );
        let relative = segment_to_text(
            &segment,
            current,
            subpath_start,
            decimals,
            minify,
            true,
            flags,
            last_cubic_ctrl,
            last_quad_ctrl,
        );
        let text = if flags.get("useRelativeAbsolute").copied().unwrap_or(false) {
            if relative.len() < absolute.len() {
                relative
            } else {
                absolute
            }
        } else if relative_mode {
            relative
        } else {
            absolute
        };
        if !text.is_empty() {
            parts.push(text);
        }
        match (&segment.cmd, &segment.values) {
            ('M', _) => {
                current = segment.end;
                subpath_start = segment.end;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            ('Z', _) => {
                current = subpath_start;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
            ('C', SegmentValues::Cubic(_, c2)) => {
                current = segment.end;
                last_cubic_ctrl = Some(*c2);
                last_quad_ctrl = None;
            }
            ('Q', SegmentValues::Quad(c)) => {
                current = segment.end;
                last_quad_ctrl = Some(*c);
                last_cubic_ctrl = None;
            }
            _ => {
                current = segment.end;
                last_cubic_ctrl = None;
                last_quad_ctrl = None;
            }
        }
    }
    let joiner = if minify { "" } else { " " };
    parts.join(joiner)
}

#[allow(clippy::too_many_arguments)]
fn segment_to_text(
    segment: &Segment,
    current: Point,
    _subpath_start: Point,
    decimals: usize,
    minify: bool,
    relative: bool,
    flags: &HashMap<String, bool>,
    last_cubic_ctrl: Option<Point>,
    last_quad_ctrl: Option<Point>,
) -> String {
    match (&segment.cmd, &segment.values) {
        ('M', _) => {
            let point = if relative { delta(segment.end, current) } else { segment.end };
            command_text(if relative { 'm' } else { 'M' }, &[point.x, point.y], decimals, minify)
        }
        ('Z', _) => {
            if relative { "z" } else { "Z" }.to_string()
        }
        ('L', _) => {
            let end = if relative { delta(segment.end, current) } else { segment.end };
            if flags
                .get("useHorizontalAndVerticalLines")
                .copied()
                .unwrap_or(false)
            {
                if (segment.end.y - current.y).abs() <= 1e-9 {
                    return command_text(
                        if relative { 'h' } else { 'H' },
                        &[if relative { end.x } else { segment.end.x }],
                        decimals,
                        minify,
                    );
                }
                if (segment.end.x - current.x).abs() <= 1e-9 {
                    return command_text(
                        if relative { 'v' } else { 'V' },
                        &[if relative { end.y } else { segment.end.y }],
                        decimals,
                        minify,
                    );
                }
            }
            command_text(if relative { 'l' } else { 'L' }, &[end.x, end.y], decimals, minify)
        }
        ('C', SegmentValues::Cubic(c1, c2)) => {
            if flags.get("useShorthands").copied().unwrap_or(false) {
                if let Some(last) = last_cubic_ctrl {
                    let reflected = Point::new(2.0 * current.x - last.x, 2.0 * current.y - last.y);
                    if c1.close_to(reflected) {
                        let p2 = if relative { delta(*c2, current) } else { *c2 };
                        let p = if relative { delta(segment.end, current) } else { segment.end };
                        return command_text(
                            if relative { 's' } else { 'S' },
                            &[p2.x, p2.y, p.x, p.y],
                            decimals,
                            minify,
                        );
                    }
                }
            }
            let vals = if relative {
                let c1 = delta(*c1, current);
                let c2 = delta(*c2, current);
                let end = delta(segment.end, current);
                vec![c1.x, c1.y, c2.x, c2.y, end.x, end.y]
            } else {
                vec![c1.x, c1.y, c2.x, c2.y, segment.end.x, segment.end.y]
            };
            command_text(if relative { 'c' } else { 'C' }, &vals, decimals, minify)
        }
        ('Q', SegmentValues::Quad(c)) => {
            if flags.get("useShorthands").copied().unwrap_or(false) {
                if let Some(last) = last_quad_ctrl {
                    let reflected = Point::new(2.0 * current.x - last.x, 2.0 * current.y - last.y);
                    if c.close_to(reflected) {
                        let p = if relative { delta(segment.end, current) } else { segment.end };
                        return command_text(
                            if relative { 't' } else { 'T' },
                            &[p.x, p.y],
                            decimals,
                            minify,
                        );
                    }
                }
            }
            let vals = if relative {
                let c = delta(*c, current);
                let end = delta(segment.end, current);
                vec![c.x, c.y, end.x, end.y]
            } else {
                vec![c.x, c.y, segment.end.x, segment.end.y]
            };
            command_text(if relative { 'q' } else { 'Q' }, &vals, decimals, minify)
        }
        (
            'A',
            SegmentValues::Arc {
                rx,
                ry,
                rotation,
                large_arc,
                sweep,
            },
        ) => {
            let end = if relative { delta(segment.end, current) } else { segment.end };
            command_text(
                if relative { 'a' } else { 'A' },
                &[*rx, *ry, *rotation, *large_arc as f64, *sweep as f64, end.x, end.y],
                decimals,
                minify,
            )
        }
        _ => String::new(),
    }
}

fn delta(point: Point, origin: Point) -> Point {
    Point::new(point.x - origin.x, point.y - origin.y)
}

fn command_text(command: char, numbers: &[f64], decimals: usize, minify: bool) -> String {
    if numbers.is_empty() {
        return command.to_string();
    }
    let formatted: Vec<_> = numbers.iter().map(|n| fmt_number(*n, decimals, minify)).collect();
    if !minify {
        return format!("{}{}", command, formatted.join(" "));
    }
    let mut text = command.to_string();
    let mut previous = String::new();
    for number in formatted {
        if previous.is_empty()
            || number.starts_with('-')
            || (number.starts_with('.') && previous.chars().last().is_some_and(|c| c.is_ascii_digit()))
        {
            text.push_str(&number);
        } else {
            text.push(' ');
            text.push_str(&number);
        }
        previous = number;
    }
    text
}

fn fmt_number(value: f64, decimals: usize, minify: bool) -> String {
    let mut v = value;
    if v.abs() < 10f64.powi(-((decimals as i32) + 1)) {
        v = 0.0;
    }
    let rounded = round_to(v, decimals);
    let mut text = format!("{:.*}", decimals, rounded);
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text.is_empty() || text == "-0" {
        text = "0".to_string();
    }
    if minify {
        if let Some(rest) = text.strip_prefix("0.") {
            text = format!(".{}", rest);
        } else if let Some(rest) = text.strip_prefix("-0.") {
            text = format!(".{}", rest);
            text.insert(0, '-');
        }
    }
    text
}

fn round_to(value: f64, decimals: usize) -> f64 {
    let factor = 10f64.powi(decimals as i32);
    (value * factor).round() / factor
}

fn angle_between(ux: f64, uy: f64, vx: f64, vy: f64) -> f64 {
    (ux * vy - uy * vx).atan2(ux * vx + uy * vy)
}

fn arc_to_center(
    p0: Point,
    mut rx: f64,
    mut ry: f64,
    x_axis_rotation: f64,
    large_arc: i32,
    sweep: i32,
    p1: Point,
) -> Result<(Point, f64, f64, f64, f64, f64)> {
    rx = rx.abs();
    ry = ry.abs();
    if rx == 0.0 || ry == 0.0 || p0.close_to(p1) {
        return Err("Degenerate arc cannot be center-parameterized".into());
    }
    let phi = (x_axis_rotation % 360.0).to_radians();
    let cos_phi = phi.cos();
    let sin_phi = phi.sin();
    let dx = (p0.x - p1.x) / 2.0;
    let dy = (p0.y - p1.y) / 2.0;
    let x1p = cos_phi * dx + sin_phi * dy;
    let y1p = -sin_phi * dx + cos_phi * dy;
    let radius_check = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if radius_check > 1.0 {
        let factor = radius_check.sqrt();
        rx *= factor;
        ry *= factor;
    }
    let sign = if large_arc == sweep { -1.0 } else { 1.0 };
    let numerator = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
    let denominator = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
    let coef = if denominator == 0.0 {
        0.0
    } else {
        sign * (numerator / denominator).max(0.0).sqrt()
    };
    let cxp = coef * (rx * y1p / ry);
    let cyp = coef * (-ry * x1p / rx);
    let cx = cos_phi * cxp - sin_phi * cyp + (p0.x + p1.x) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (p0.y + p1.y) / 2.0;
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = angle_between(1.0, 0.0, ux, uy);
    let mut delta = angle_between(ux, uy, vx, vy);
    if sweep == 0 && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    } else if sweep != 0 && delta < 0.0 {
        delta += std::f64::consts::TAU;
    }
    Ok((Point::new(cx, cy), rx, ry, phi, theta1, delta))
}

fn arc_to_cubic_segments(segment: &Segment) -> Result<Vec<Segment>> {
    let SegmentValues::Arc {
        rx,
        ry,
        rotation,
        large_arc,
        sweep,
    } = segment.values
    else {
        return Err("arc_to_cubic_segments requires an A segment".into());
    };
    let Ok((center, rx, ry, phi, theta1, delta)) =
        arc_to_center(segment.start, rx, ry, rotation, large_arc, sweep, segment.end)
    else {
        return Ok(vec![Segment::new(
            'L',
            segment.start,
            segment.end,
            SegmentValues::None,
            segment.index,
        )]);
    };
    let cos_phi = phi.cos();
    let sin_phi = phi.sin();
    let pieces = (delta.abs() / (std::f64::consts::PI / 2.0)).ceil().max(1.0) as usize;
    let step = delta / pieces as f64;
    let mut current = segment.start;
    let mut cubics = Vec::new();
    for i in 0..pieces {
        let t1 = theta1 + i as f64 * step;
        let t2 = t1 + step;
        let alpha = 4.0 / 3.0 * ((t2 - t1) / 4.0).tan();
        let point = |theta: f64| -> Point {
            Point::new(
                center.x + cos_phi * rx * theta.cos() - sin_phi * ry * theta.sin(),
                center.y + sin_phi * rx * theta.cos() + cos_phi * ry * theta.sin(),
            )
        };
        let derivative = |theta: f64| -> Point {
            Point::new(
                -cos_phi * rx * theta.sin() - sin_phi * ry * theta.cos(),
                -sin_phi * rx * theta.sin() + cos_phi * ry * theta.cos(),
            )
        };
        let p1 = point(t1);
        let p2 = point(t2);
        let d1 = derivative(t1);
        let d2 = derivative(t2);
        let c1 = Point::new(p1.x + alpha * d1.x, p1.y + alpha * d1.y);
        let c2 = Point::new(p2.x - alpha * d2.x, p2.y - alpha * d2.y);
        let end = if i == pieces - 1 { segment.end } else { p2 };
        cubics.push(Segment::new(
            'C',
            current,
            end,
            SegmentValues::Cubic(c1, c2),
            segment.index,
        ));
        current = end;
    }
    Ok(cubics)
}

fn line_to_cubic_segment(start: Point, end: Point, index: usize) -> Segment {
    let c1 = Point::new(
        start.x + (end.x - start.x) / 3.0,
        start.y + (end.y - start.y) / 3.0,
    );
    let c2 = Point::new(
        start.x + 2.0 * (end.x - start.x) / 3.0,
        start.y + 2.0 * (end.y - start.y) / 3.0,
    );
    Segment::new('C', start, end, SegmentValues::Cubic(c1, c2), index)
}

fn quadratic_to_cubic_segment(start: Point, ctrl: Point, end: Point, index: usize) -> Segment {
    let c1 = Point::new(
        start.x + 2.0 * (ctrl.x - start.x) / 3.0,
        start.y + 2.0 * (ctrl.y - start.y) / 3.0,
    );
    let c2 = Point::new(
        end.x + 2.0 * (ctrl.x - end.x) / 3.0,
        end.y + 2.0 * (ctrl.y - end.y) / 3.0,
    );
    Segment::new('C', start, end, SegmentValues::Cubic(c1, c2), index)
}

fn path_segments_to_cubics(segments: &[Segment]) -> Result<Vec<Segment>> {
    let mut result = Vec::new();
    let mut current = Point::new(0.0, 0.0);
    let mut subpath_start = Point::new(0.0, 0.0);
    for segment in recompute_starts(segments) {
        match (&segment.cmd, &segment.values) {
            ('M', _) => {
                result.push(segment.clone());
                current = segment.end;
                subpath_start = segment.end;
            }
            ('L', _) => {
                result.push(line_to_cubic_segment(current, segment.end, segment.index));
                current = segment.end;
            }
            ('C', _) => {
                result.push(segment.clone());
                current = segment.end;
            }
            ('Q', SegmentValues::Quad(ctrl)) => {
                result.push(quadratic_to_cubic_segment(
                    segment.start,
                    *ctrl,
                    segment.end,
                    segment.index,
                ));
                current = segment.end;
            }
            ('A', _) => {
                let cubics = arc_to_cubic_segments(&segment)?;
                current = cubics.last().map(|c| c.end).unwrap_or(segment.end);
                result.extend(cubics);
            }
            ('Z', _) => {
                if !current.close_to(subpath_start) {
                    result.push(line_to_cubic_segment(current, subpath_start, segment.index));
                }
                result.push(Segment::new(
                    'Z',
                    subpath_start,
                    subpath_start,
                    SegmentValues::None,
                    segment.index,
                ));
                current = subpath_start;
            }
            _ => return Err(SvgoError(format!("Unsupported segment for cubic conversion: {}", segment.cmd))),
        }
    }
    Ok(reindex_segments(recompute_starts(&result)))
}

fn split_operation(operation: &str) -> (String, String) {
    if let Some(raw) = operation.strip_prefix("matrix(").and_then(|s| s.strip_suffix(')')) {
        return ("matrix".to_string(), raw.to_string());
    }
    if let Some((name, rest)) = operation.split_once(':') {
        (name.to_string(), rest.to_string())
    } else {
        (operation.to_string(), String::new())
    }
}

fn parse_number_list(text: &str, expected: usize, label: &str) -> Result<Vec<f64>> {
    let parts: Vec<_> = text.split(',').filter(|p| !p.trim().is_empty()).collect();
    if parts.len() != expected {
        return Err(SvgoError(format!(
            "{} requires {} comma-separated numbers",
            label, expected
        )));
    }
    parts
        .iter()
        .map(|p| {
            p.trim()
                .parse::<f64>()
                .map_err(|_| SvgoError(format!("{} contains an invalid number", label)))
        })
        .collect()
}

fn parse_non_negative_int(text: &str, label: &str) -> Result<usize> {
    let value = text
        .parse::<isize>()
        .map_err(|_| SvgoError(format!("{} must be a non-negative integer: {}", label, text)))?;
    if value < 0 {
        return Err(SvgoError(format!(
            "{} must be a non-negative integer: {}",
            label, text
        )));
    }
    Ok(value as usize)
}

#[pyfunction]
fn parse_path_json(path_data: &str) -> PyResult<String> {
    Ok(PathDataCore::parse(path_data)
        .map_err(py_err)?
        .command_items()
        .to_string())
}

#[pyfunction]
#[pyo3(signature = (path_data, decimals=4, minify=false))]
fn path_to_absolute(path_data: &str, decimals: usize, minify: bool) -> PyResult<String> {
    let mut path = PathDataCore::parse(path_data).map_err(py_err)?;
    path.set_relative(false);
    Ok(path.to_string(decimals, minify))
}

#[pyfunction]
#[pyo3(signature = (path_data, decimals=4, minify=false))]
fn path_to_relative(path_data: &str, decimals: usize, minify: bool) -> PyResult<String> {
    let mut path = PathDataCore::parse(path_data).map_err(py_err)?;
    path.set_relative(true);
    Ok(path.to_string(decimals, minify))
}

#[pyfunction]
#[pyo3(signature = (path_data, matrix, decimals=4, minify=false))]
fn transform_path(path_data: &str, matrix: Vec<f64>, decimals: usize, minify: bool) -> PyResult<String> {
    let mut path = PathDataCore::parse(path_data).map_err(py_err)?;
    path.transform(coerce_matrix_slice(&matrix).map_err(py_err)?).map_err(py_err)?;
    Ok(path.to_string(decimals, minify))
}

#[pyfunction]
#[pyo3(signature = (path_data, decimals=4, minify=false))]
fn path_to_cubics(path_data: &str, decimals: usize, minify: bool) -> PyResult<String> {
    let mut path = PathDataCore::parse(path_data).map_err(py_err)?;
    path.to_cubics().map_err(py_err)?;
    Ok(path.to_string(decimals, minify))
}

