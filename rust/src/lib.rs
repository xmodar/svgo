use base64::Engine;
use miniz_oxide::inflate::decompress_to_vec_zlib;
use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::io::Cursor;
use std::path::Path;
use xmltree::{Element, EmitterConfig, XMLNode};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const DATA_URI_SAFE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'#')
    .add(b'%')
    .add(b'{')
    .add(b'}')
    .add(b'|')
    .add(b'\\')
    .add(b'^')
    .add(b'`')
    .add(b'[')
    .add(b']');

type Matrix = [f64; 6];
const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Debug, Clone)]
struct SvgoError(String);

impl std::fmt::Display for SvgoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for SvgoError {}

impl From<&str> for SvgoError {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

impl From<String> for SvgoError {
    fn from(value: String) -> Self {
        Self(value)
    }
}

type Result<T> = std::result::Result<T, SvgoError>;

fn py_err(err: SvgoError) -> PyErr {
    PyValueError::new_err(err.0)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Point {
    x: f64,
    y: f64,
}

impl Point {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn transform(self, matrix: Matrix) -> Self {
        let [a, b, c, d, e, f] = matrix;
        Self {
            x: a * self.x + c * self.y + e,
            y: b * self.x + d * self.y + f,
        }
    }

    fn close_to(self, other: Point) -> bool {
        (self.x - other.x).abs() <= 1e-9 && (self.y - other.y).abs() <= 1e-9
    }
}

#[derive(Debug, Clone)]
enum SegmentValues {
    None,
    Cubic(Point, Point),
    Quad(Point),
    Arc {
        rx: f64,
        ry: f64,
        rotation: f64,
        large_arc: i32,
        sweep: i32,
    },
}

#[derive(Debug, Clone)]
struct Segment {
    cmd: char,
    start: Point,
    end: Point,
    values: SegmentValues,
    index: usize,
}

impl Segment {
    fn new(cmd: char, start: Point, end: Point, values: SegmentValues, index: usize) -> Self {
        Self {
            cmd,
            start,
            end,
            values,
            index,
        }
    }

    fn reversed(&self) -> Result<Self> {
        match (&self.cmd, &self.values) {
            ('M', _) => Ok(Self::new('M', self.end, self.end, SegmentValues::None, self.index)),
            ('L', _) => Ok(Self::new('L', self.end, self.start, SegmentValues::None, self.index)),
            ('C', SegmentValues::Cubic(c1, c2)) => Ok(Self::new(
                'C',
                self.end,
                self.start,
                SegmentValues::Cubic(*c2, *c1),
                self.index,
            )),
            ('Q', SegmentValues::Quad(c)) => Ok(Self::new(
                'Q',
                self.end,
                self.start,
                SegmentValues::Quad(*c),
                self.index,
            )),
            (
                'A',
                SegmentValues::Arc {
                    rx,
                    ry,
                    rotation,
                    large_arc,
                    sweep,
                },
            ) => Ok(Self::new(
                'A',
                self.end,
                self.start,
                SegmentValues::Arc {
                    rx: *rx,
                    ry: *ry,
                    rotation: *rotation,
                    large_arc: *large_arc,
                    sweep: if *sweep == 0 { 1 } else { 0 },
                },
                self.index,
            )),
            ('Z', _) => Ok(Self::new('Z', self.end, self.start, SegmentValues::None, self.index)),
            _ => Err(SvgoError(format!(
                "Unsupported segment for reverse: {}",
                self.cmd
            ))),
        }
    }

    fn transformed(&self, matrix: Matrix) -> Result<Vec<Self>> {
        match (&self.cmd, &self.values) {
            ('M', _) => {
                let p = self.end.transform(matrix);
                Ok(vec![Self::new('M', p, p, SegmentValues::None, self.index)])
            }
            ('L', _) => Ok(vec![Self::new(
                'L',
                self.start.transform(matrix),
                self.end.transform(matrix),
                SegmentValues::None,
                self.index,
            )]),
            ('C', SegmentValues::Cubic(c1, c2)) => Ok(vec![Self::new(
                'C',
                self.start.transform(matrix),
                self.end.transform(matrix),
                SegmentValues::Cubic(c1.transform(matrix), c2.transform(matrix)),
                self.index,
            )]),
            ('Q', SegmentValues::Quad(c)) => Ok(vec![Self::new(
                'Q',
                self.start.transform(matrix),
                self.end.transform(matrix),
                SegmentValues::Quad(c.transform(matrix)),
                self.index,
            )]),
            ('A', _) => {
                let mut out = Vec::new();
                for cubic in arc_to_cubic_segments(self)? {
                    out.extend(cubic.transformed(matrix)?);
                }
                Ok(out)
            }
            ('Z', _) => Ok(vec![Self::new(
                'Z',
                self.start.transform(matrix),
                self.end.transform(matrix),
                SegmentValues::None,
                self.index,
            )]),
            _ => Err(SvgoError(format!(
                "Unsupported segment for transform: {}",
                self.cmd
            ))),
        }
    }
}

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

fn get_kappa() -> f64 {
    4.0 * (2.0_f64.sqrt() - 1.0) / 3.0
}

fn rect_to_path_core(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    rx: f64,
    ry: Option<f64>,
    decimals: usize,
    minify: bool,
) -> Result<String> {
    if width < 0.0 || height < 0.0 {
        return Err("rect width and height must be non-negative".into());
    }
    if width == 0.0 || height == 0.0 {
        return Ok(String::new());
    }
    let rx = rx.max(0.0).min(width / 2.0);
    let ry = ry.unwrap_or(rx).max(0.0).min(height / 2.0);
    if rx == 0.0 && ry == 0.0 {
        return Ok(PathDataCore::parse(&format!(
            "M{} {}L{} {}L{} {}L{} {}Z",
            x,
            y,
            x + width,
            y,
            x + width,
            y + height,
            x,
            y + height
        ))?
        .to_string(decimals, minify));
    }
    let k = get_kappa();
    let ox = rx * k;
    let oy = ry * k;
    let raw = format!(
        "M{} {}L{} {}C{} {} {} {} {} {}L{} {}C{} {} {} {} {} {}L{} {}C{} {} {} {} {} {}L{} {}C{} {} {} {} {} {}Z",
        x + rx,
        y,
        x + width - rx,
        y,
        x + width - rx + ox,
        y,
        x + width,
        y + ry - oy,
        x + width,
        y + ry,
        x + width,
        y + height - ry,
        x + width,
        y + height - ry + oy,
        x + width - rx + ox,
        y + height,
        x + width - rx,
        y + height,
        x + rx,
        y + height,
        x + rx - ox,
        y + height,
        x,
        y + height - ry + oy,
        x,
        y + height - ry,
        x,
        y + ry,
        x,
        y + ry - oy,
        x + rx - ox,
        y,
        x + rx,
        y
    );
    Ok(PathDataCore::parse(&raw)?.to_string(decimals, minify))
}

fn ellipse_to_path_core(
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    decimals: usize,
    minify: bool,
) -> Result<String> {
    if rx < 0.0 || ry < 0.0 {
        return Err("ellipse radii must be non-negative".into());
    }
    if rx == 0.0 || ry == 0.0 {
        return Ok(String::new());
    }
    let ox = rx * get_kappa();
    let oy = ry * get_kappa();
    let raw = format!(
        "M{} {}C{} {} {} {} {} {}C{} {} {} {} {} {}C{} {} {} {} {} {}C{} {} {} {} {} {}Z",
        cx,
        cy - ry,
        cx + ox,
        cy - ry,
        cx + rx,
        cy - oy,
        cx + rx,
        cy,
        cx + rx,
        cy + oy,
        cx + ox,
        cy + ry,
        cx,
        cy + ry,
        cx - ox,
        cy + ry,
        cx - rx,
        cy + oy,
        cx - rx,
        cy,
        cx - rx,
        cy - oy,
        cx - ox,
        cy - ry,
        cx,
        cy - ry
    );
    Ok(PathDataCore::parse(&raw)?.to_string(decimals, minify))
}

fn parse_points(points: &str) -> Vec<(f64, f64)> {
    let values = parse_float_list(points);
    values
        .chunks(2)
        .filter(|pair| pair.len() == 2)
        .map(|pair| (pair[0], pair[1]))
        .collect()
}

#[pyfunction]
fn translate_2d(tx: f64, ty: Option<f64>) -> Vec<f64> {
    translate_matrix(tx, ty.unwrap_or(0.0)).to_vec()
}

#[pyfunction]
fn scale_2d(sx: f64, sy: Option<f64>) -> Vec<f64> {
    scale_matrix(sx, sy.unwrap_or(sx)).to_vec()
}

#[pyfunction]
#[pyo3(signature = (angle, cx=0.0, cy=0.0, degrees=false))]
fn rotate_2d(angle: f64, cx: f64, cy: f64, degrees: bool) -> Vec<f64> {
    rotate_matrix(cx, cy, if degrees { angle } else { angle.to_degrees() }).to_vec()
}

#[pyfunction]
fn multiply_matrices(left: Vec<f64>, right: Vec<f64>) -> PyResult<Vec<f64>> {
    Ok(matrix_multiply(
        coerce_matrix_slice(&left).map_err(py_err)?,
        coerce_matrix_slice(&right).map_err(py_err)?,
    )
    .to_vec())
}

#[pyfunction]
fn transform_2d(matrix: Vec<f64>, x: f64, y: f64) -> PyResult<(f64, f64)> {
    let [a, b, c, d, e, f] = coerce_matrix_slice(&matrix).map_err(py_err)?;
    Ok((a * x + c * y + e, b * x + d * y + f))
}

#[pyfunction]
fn matrix_to_3x3(matrix: Vec<f64>) -> PyResult<Vec<Vec<f64>>> {
    let [a, b, c, d, e, f] = coerce_matrix_slice(&matrix).map_err(py_err)?;
    Ok(vec![vec![a, c, e], vec![b, d, f], vec![0.0, 0.0, 1.0]])
}

#[pyfunction]
fn identity(size: usize) -> PyResult<Vec<Vec<f64>>> {
    if size == 0 {
        return Err(PyValueError::new_err("identity size must be positive"));
    }
    Ok((0..size)
        .map(|row| {
            (0..size)
                .map(|col| if row == col { 1.0 } else { 0.0 })
                .collect()
        })
        .collect())
}

#[pyfunction]
#[pyo3(signature = (x=0.0, y=0.0, width=0.0, height=0.0, rx=0.0, ry=None, decimals=20, minify=false))]
fn rect_to_path(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    rx: f64,
    ry: Option<f64>,
    decimals: usize,
    minify: bool,
) -> PyResult<String> {
    rect_to_path_core(x, y, width, height, rx, ry, decimals, minify).map_err(py_err)
}

#[pyfunction]
#[pyo3(signature = (x1, y1, x2, y2, decimals=20, minify=false))]
fn line_to_path(x1: f64, y1: f64, x2: f64, y2: f64, decimals: usize, minify: bool) -> PyResult<String> {
    Ok(PathDataCore::parse(&format!("M{} {}L{} {}", x1, y1, x2, y2))
        .map_err(py_err)?
        .to_string(decimals, minify))
}

#[pyfunction]
#[pyo3(signature = (cx, cy, r, decimals=20, minify=false))]
fn circle_to_path(cx: f64, cy: f64, r: f64, decimals: usize, minify: bool) -> PyResult<String> {
    if r < 0.0 {
        return Err(PyValueError::new_err("circle radius must be non-negative"));
    }
    ellipse_to_path_core(cx, cy, r, r, decimals, minify).map_err(py_err)
}

#[pyfunction]
#[pyo3(signature = (cx, cy, rx, ry, decimals=20, minify=false))]
fn ellipse_to_path(cx: f64, cy: f64, rx: f64, ry: f64, decimals: usize, minify: bool) -> PyResult<String> {
    ellipse_to_path_core(cx, cy, rx, ry, decimals, minify).map_err(py_err)
}

#[pyfunction]
#[pyo3(signature = (points_json, decimals=20, minify=false))]
fn polyline_to_path(points_json: &str, decimals: usize, minify: bool) -> PyResult<String> {
    let points: Vec<Vec<f64>> = serde_json::from_str(points_json)
        .map_err(|e| PyValueError::new_err(format!("points must be JSON pairs: {}", e)))?;
    let pairs: Vec<_> = points
        .into_iter()
        .map(|p| {
            if p.len() != 2 {
                Err(PyValueError::new_err("points must contain two numeric values"))
            } else {
                Ok((p[0], p[1]))
            }
        })
        .collect::<PyResult<Vec<_>>>()?;
    if pairs.is_empty() {
        return Ok(String::new());
    }
    let raw = format!(
        "M{}",
        pairs
            .iter()
            .map(|(x, y)| format!("{} {}", x, y))
            .collect::<Vec<_>>()
            .join("L")
    );
    Ok(PathDataCore::parse(&raw).map_err(py_err)?.to_string(decimals, minify))
}

#[pyfunction]
#[pyo3(signature = (points_json, decimals=20, minify=false))]
fn polygon_to_path(points_json: &str, decimals: usize, minify: bool) -> PyResult<String> {
    let mut path = polyline_to_path(points_json, decimals, minify)?;
    if !path.is_empty() {
        if minify {
            path.push('Z');
        } else {
            path.push_str(" Z");
        }
    }
    Ok(path)
}

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

fn parse_plugin_spec_cli(spec: &str) -> Result<PluginSpecCore> {
    if let Some((name, raw)) = spec.split_once(':') {
        let params: Value = serde_json::from_str(raw)
            .map_err(|e| SvgoError(format!("Invalid JSON params for plugin {}: {}", name, e)))?;
        if !params.is_object() {
            return Err(SvgoError(format!("Plugin params for {} must be a JSON object", name)));
        }
        Ok(PluginSpecCore { name: name.to_string(), params })
    } else {
        Ok(plugin(spec))
    }
}

#[derive(Default)]
struct ArgCursor {
    args: Vec<String>,
    i: usize,
}

impl ArgCursor {
    fn new(args: &[String]) -> Self {
        Self { args: args.to_vec(), i: 0 }
    }

    fn next(&mut self) -> Option<String> {
        let value = self.args.get(self.i).cloned();
        if value.is_some() {
            self.i += 1;
        }
        value
    }

    fn value(&mut self, option: &str) -> Result<String> {
        self.next().ok_or_else(|| SvgoError(format!("{} requires a value", option)))
    }
}

fn write_file_or_return(text: String, output: Option<String>) -> Result<String> {
    if let Some(output) = output {
        fs::write(&output, if text.ends_with('\n') { text.clone() } else { format!("{}\n", text) })
            .map_err(|e| SvgoError(e.to_string()))?;
        Ok(String::new())
    } else {
        Ok(text)
    }
}

fn read_file(path: &str) -> Result<String> {
    fs::read_to_string(path).map_err(|e| SvgoError(e.to_string()))
}

fn plugin_list_text() -> String {
    builtin_plugins()
        .into_iter()
        .map(|name| format!("{}\t{}", name, if name == "preset-default" { "preset" } else { "plugin" }))
        .collect::<Vec<_>>()
        .join("\n")
}

fn build_opt_options_from_pairs(
    preset: String,
    plugins: Vec<PluginSpecCore>,
    disabled: Vec<String>,
    precision: Option<usize>,
    multipass: bool,
    pretty: bool,
    indent: usize,
    eol: Option<String>,
    final_newline: bool,
    datauri: Option<String>,
) -> OptimizeOptionsCore {
    OptimizeOptionsCore {
        preset,
        plugins,
        disabled,
        float_precision: precision,
        multipass,
        pretty,
        indent,
        eol,
        final_newline,
        datauri,
    }
}

fn selected_indexes(select: &str, count: usize) -> Result<HashSet<usize>> {
    if select == "all" {
        return Ok((0..count).collect());
    }
    let mut indexes = HashSet::new();
    for part in select.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let index: usize = part.parse().map_err(|_| SvgoError(format!("select index must be a non-negative integer: {}", part)))?;
        if index >= count {
            return Err(SvgoError(format!("select index {} is out of range; file has {} path d attributes", index, count)));
        }
        indexes.insert(index);
    }
    Ok(indexes)
}

fn find_d_attributes(text: &str) -> Vec<(usize, usize, char, String)> {
    let mut matches = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i + 2 < bytes.len() {
        if bytes[i] == b'd' && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < bytes.len() && (bytes[j] == b'"' || bytes[j] == b'\'') {
                    let quote = bytes[j] as char;
                    let value_start = j + 1;
                    if let Some(end_rel) = text[value_start..].find(quote) {
                        let value_end = value_start + end_rel;
                        matches.push((i, value_end + 1, quote, text[value_start..value_end].to_string()));
                        i = value_end + 1;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    matches
}

fn edit_path_data(path_data: &str, ops: &[String], decimals: usize, minify: bool, svgo_options: Option<&OptimizeOptionsCore>, svgo_before: bool, svgo_after: bool) -> Result<String> {
    let mut result = path_data.trim().to_string();
    if svgo_before {
        result = optimize_path_data(&result, svgo_options.cloned().unwrap_or_default())?;
    }
    let mut path = PathDataCore::parse(&result)?;
    for op in ops {
        path.apply_operation(op, decimals)?;
    }
    result = path.to_string(decimals, minify);
    if svgo_after {
        result = optimize_path_data(&result, svgo_options.cloned().unwrap_or_default())?;
    }
    Ok(result)
}

fn optimize_path_data(path_data: &str, options: OptimizeOptionsCore) -> Result<String> {
    if options.datauri.is_some() {
        return Err("--svgo-datauri cannot be used when optimizing raw path d data".into());
    }
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="{}"/></svg>"#, escape_attr(path_data));
    let optimized = optimize_svg_core(&svg, options)?;
    let matches = find_d_attributes(&optimized);
    matches
        .first()
        .map(|(_, _, _, value)| value.clone())
        .ok_or_else(|| "Optimizer removed or could not return a path d attribute".into())
}

fn edit_svg_path_attributes(text: &str, select: &str, ops: &[String], decimals: usize, minify: bool) -> Result<String> {
    let matches = find_d_attributes(text);
    if matches.is_empty() && !ops.is_empty() {
        return Err("No path d attributes found in SVG input".into());
    }
    let selected = selected_indexes(select, matches.len())?;
    let mut output = String::new();
    let mut cursor = 0usize;
    for (index, (start, end, quote, value)) in matches.iter().enumerate() {
        output.push_str(&text[cursor..*start]);
        if selected.contains(&index) && !ops.is_empty() {
            let edited = edit_path_data(value, ops, decimals, minify, None, false, false)?;
            output.push_str(&format!("d={}{}{}", quote, edited, quote));
        } else {
            output.push_str(&text[*start..*end]);
        }
        cursor = *end;
    }
    output.push_str(&text[cursor..]);
    Ok(output)
}

fn edit_svg_text(text: &str, select: &str, ops: &[String], decimals: usize, minify: bool, svgo_options: Option<OptimizeOptionsCore>, svgo_order: &str, svgo: bool) -> Result<String> {
    let mut result = text.to_string();
    if svgo && svgo_order == "before" {
        result = optimize_svg_core(&result, svgo_options.clone().unwrap_or_default())?;
    }
    result = edit_svg_path_attributes(&result, select, ops, decimals, minify)?;
    if svgo && svgo_order == "after" {
        result = optimize_svg_core(&result, svgo_options.unwrap_or_default())?;
    }
    Ok(result)
}

fn parse_common_svgo_options(cursor: &mut ArgCursor, current: String, state: &mut SvgoCliState) -> Result<()> {
    match current.as_str() {
        "--svgo" => state.svgo = true,
        "--svgo-order" => state.svgo_order = cursor.value("--svgo-order")?,
        "--svgo-preset" => state.svgo_preset = cursor.value("--svgo-preset")?,
        "--svgo-plugin" => state.svgo_plugins.push(parse_plugin_spec_cli(&cursor.value("--svgo-plugin")?)?),
        "--svgo-disable" => state.svgo_disabled.push(cursor.value("--svgo-disable")?),
        "--svgo-precision" => state.svgo_precision = Some(cursor.value("--svgo-precision")?.parse().map_err(|_| SvgoError("--svgo-precision must be an integer".to_string()))?),
        "--svgo-multipass" => state.svgo_multipass = true,
        "--svgo-pretty" => state.svgo_pretty = true,
        "--svgo-indent" => state.svgo_indent = cursor.value("--svgo-indent")?.parse().map_err(|_| SvgoError("--svgo-indent must be an integer".to_string()))?,
        "--svgo-eol" => state.svgo_eol = Some(cursor.value("--svgo-eol")?),
        "--svgo-final-newline" => state.svgo_final_newline = true,
        "--svgo-datauri" => state.svgo_datauri = Some(cursor.value("--svgo-datauri")?),
        "--svgo-list-plugins" => state.svgo_list_plugins = true,
        "--svgo-config" => {
            let _ = cursor.value("--svgo-config")?;
        }
        _ => return Err(SvgoError(format!("unknown option {}", current))),
    }
    Ok(())
}

#[derive(Default)]
struct SvgoCliState {
    svgo: bool,
    svgo_order: String,
    svgo_preset: String,
    svgo_plugins: Vec<PluginSpecCore>,
    svgo_disabled: Vec<String>,
    svgo_precision: Option<usize>,
    svgo_multipass: bool,
    svgo_pretty: bool,
    svgo_indent: usize,
    svgo_eol: Option<String>,
    svgo_final_newline: bool,
    svgo_datauri: Option<String>,
    svgo_list_plugins: bool,
}

impl SvgoCliState {
    fn new() -> Self {
        Self {
            svgo_order: "after".to_string(),
            svgo_preset: "default".to_string(),
            svgo_indent: 2,
            ..Default::default()
        }
    }

    fn options(&self) -> OptimizeOptionsCore {
        build_opt_options_from_pairs(
            self.svgo_preset.clone(),
            self.svgo_plugins.clone(),
            self.svgo_disabled.clone(),
            self.svgo_precision,
            self.svgo_multipass,
            self.svgo_pretty,
            self.svgo_indent,
            self.svgo_eol.clone(),
            self.svgo_final_newline,
            self.svgo_datauri.clone(),
        )
    }
}

fn run_path_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut path = None;
    let mut input = None;
    let mut output = None;
    let mut select = "all".to_string();
    let mut ops = Vec::new();
    let mut decimals = 4usize;
    let mut minify = false;
    let mut svgo = SvgoCliState::new();
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--path" => path = Some(cursor.value("--path")?),
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--select" => select = cursor.value("--select")?,
            "--op" => ops.push(cursor.value("--op")?),
            "--decimals" => decimals = cursor.value("--decimals")?.parse().map_err(|_| SvgoError("--decimals must be an integer".to_string()))?,
            "--minify" => minify = true,
            _ => parse_common_svgo_options(&mut cursor, arg, &mut svgo)?,
        }
    }
    if svgo.svgo_list_plugins {
        return Ok(plugin_list_text());
    }
    if path.is_some() && input.is_some() {
        return Err("Use either --path or --input, not both".into());
    }
    let result = if let Some(path) = path {
        edit_path_data(
            &path,
            &ops,
            decimals,
            minify,
            Some(&svgo.options()),
            svgo.svgo && svgo.svgo_order == "before",
            svgo.svgo && svgo.svgo_order == "after",
        )?
    } else if let Some(input) = input {
        let text = read_file(&input)?;
        if contains_svg_markup(&text) {
            edit_svg_text(&text, &select, &ops, decimals, minify, Some(svgo.options()), &svgo.svgo_order, svgo.svgo)?
        } else {
            edit_path_data(
                text.trim(),
                &ops,
                decimals,
                minify,
                Some(&svgo.options()),
                svgo.svgo && svgo.svgo_order == "before",
                svgo.svgo && svgo.svgo_order == "after",
            )?
        }
    } else {
        return Err("Provide --path or --input".into());
    };
    write_file_or_return(result, output)
}

fn contains_svg_markup(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    ["<path", "<svg", "<rect", "<circle", "<ellipse", "<line", "<polyline", "<polygon"]
        .iter()
        .any(|needle| lower.contains(needle))
}

fn run_opt_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut input = None;
    let mut output = None;
    let mut svgo = SvgoCliState::new();
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            _ => parse_common_svgo_options(&mut cursor, arg, &mut svgo)?,
        }
    }
    if svgo.svgo_list_plugins {
        return Ok(plugin_list_text());
    }
    let input = input.ok_or_else(|| SvgoError("--input is required".to_string()))?;
    let result = optimize_svg_core(&read_file(&input)?, svgo.options())?;
    write_file_or_return(result, output)
}

fn run_trace_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut input = None;
    let mut output = None;
    let mut components_json = false;
    let mut options = TraceOptionsCore::default();
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--components-json" => components_json = true,
            "--mode" => options.mode = cursor.value("--mode")?,
            "--curve-mode" => options.curve_mode = cursor.value("--curve-mode")?,
            "--alpha-threshold" => options.alpha_threshold = cursor.value("--alpha-threshold")?.parse().map_err(|_| SvgoError("--alpha-threshold must be an integer".to_string()))?,
            "--white-threshold" => options.white_threshold = cursor.value("--white-threshold")?.parse().map_err(|_| SvgoError("--white-threshold must be an integer".to_string()))?,
            "--drop-white" => options.drop_white = true,
            "--quantize" => options.quantize = cursor.value("--quantize")?.parse().map_err(|_| SvgoError("--quantize must be an integer".to_string()))?,
            "--max-colors" => options.max_colors = cursor.value("--max-colors")?.parse().map_err(|_| SvgoError("--max-colors must be an integer".to_string()))?,
            "--min-area" => options.min_area = cursor.value("--min-area")?.parse().map_err(|_| SvgoError("--min-area must be an integer".to_string()))?,
            "--scale" => options.scale = cursor.value("--scale")?.parse().map_err(|_| SvgoError("--scale must be a number".to_string()))?,
            "--decimals" => options.decimals = cursor.value("--decimals")?.parse().map_err(|_| SvgoError("--decimals must be an integer".to_string()))?,
            "--title" => options.title = Some(cursor.value("--title")?),
            "--palette" => {
                options.palette = cursor
                    .value("--palette")?
                    .split(',')
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_string)
                    .collect()
            }
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let input = input.ok_or_else(|| SvgoError("--input is required".to_string()))?;
    let image = read_png(Path::new(&input))?;
    if components_json {
        write_file_or_return(trace_components_value(&image, options)?.to_string(), output)
    } else {
        write_file_or_return(trace_image_core(&image, options)?, output)
    }
}

fn run_trace2_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut input = None;
    let mut output = None;
    let mut options = VTracerOptionsCore::default();
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--color-mode" => options.color_mode = cursor.value("--color-mode")?,
            "--hierarchical" | "--clustering" => options.hierarchical = cursor.value("--hierarchical")?,
            "--color-precision" => options.color_precision = cursor.value("--color-precision")?.parse().map_err(|_| SvgoError("--color-precision must be an integer".to_string()))?,
            "--gradient-step" => options.gradient_step = cursor.value("--gradient-step")?.parse().map_err(|_| SvgoError("--gradient-step must be an integer".to_string()))?,
            "--filter-speckle" => options.filter_speckle = cursor.value("--filter-speckle")?.parse().map_err(|_| SvgoError("--filter-speckle must be an integer".to_string()))?,
            "--curve-mode" => options.curve_mode = cursor.value("--curve-mode")?,
            "--corner-threshold" => options.corner_threshold = cursor.value("--corner-threshold")?.parse().map_err(|_| SvgoError("--corner-threshold must be an integer".to_string()))?,
            "--segment-length" => options.segment_length = cursor.value("--segment-length")?.parse().map_err(|_| SvgoError("--segment-length must be a number".to_string()))?,
            "--max-iterations" => options.max_iterations = cursor.value("--max-iterations")?.parse().map_err(|_| SvgoError("--max-iterations must be an integer".to_string()))?,
            "--splice-threshold" => options.splice_threshold = cursor.value("--splice-threshold")?.parse().map_err(|_| SvgoError("--splice-threshold must be an integer".to_string()))?,
            "--path-precision" => options.path_precision = cursor.value("--path-precision")?.parse().map_err(|_| SvgoError("--path-precision must be an integer".to_string()))?,
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let input = input.ok_or_else(|| SvgoError("--input is required".to_string()))?;
    parse_vtracer_options(Some(&serde_json::to_string(&options).unwrap()))?;
    let image = match read_png(Path::new(&input)) {
        Ok(image) => image,
        Err(err) => {
            if fs::read(&input).is_ok_and(|data| data.starts_with(PNG_SIGNATURE)) {
                return write_file_or_return(
                    r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#.to_string(),
                    output,
                );
            }
            return Err(err);
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
    write_file_or_return(trace_image_core(&image, trace_options)?, output)
}

fn run_center_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut path = None;
    let mut input = None;
    let mut output = None;
    let mut options = CenterlineOptionsCore::default();
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--path" => path = Some(cursor.value("--path")?),
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--emit" => options.emit = cursor.value("--emit")?,
            "--mode" => options.mode = cursor.value("--mode")?,
            "--scale" => options.scale = cursor.value("--scale")?.parse().map_err(|_| SvgoError("--scale must be a number".to_string()))?,
            "--max-size" => options.max_size = cursor.value("--max-size")?.parse().map_err(|_| SvgoError("--max-size must be an integer".to_string()))?,
            "--curve-samples" => options.curve_samples = cursor.value("--curve-samples")?.parse().map_err(|_| SvgoError("--curve-samples must be an integer".to_string()))?,
            "--simplify" => options.simplify = cursor.value("--simplify")?.parse().map_err(|_| SvgoError("--simplify must be a number".to_string()))?,
            "--min-length" => options.min_length = cursor.value("--min-length")?.parse().map_err(|_| SvgoError("--min-length must be a number".to_string()))?,
            "--stroke-width" => options.stroke_width = cursor.value("--stroke-width")?,
            "--linecap" => options.linecap = cursor.value("--linecap")?,
            "--linejoin" => options.linejoin = cursor.value("--linejoin")?,
            "--decimals" => options.decimals = cursor.value("--decimals")?.parse().map_err(|_| SvgoError("--decimals must be an integer".to_string()))?,
            "--polyline" => options.polyline = true,
            "--fill-rule" => options.fill_rule = cursor.value("--fill-rule")?,
            "--svg-paths" => options.svg_paths = cursor.value("--svg-paths")?,
            "--keep-failed" => options.keep_failed = true,
            "--bridge-gap" => options.bridge_gap = cursor.value("--bridge-gap")?.parse().map_err(|_| SvgoError("--bridge-gap must be a number".to_string()))?,
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let result = if let Some(path) = path {
        let (d, stroke_width, ctx) = centerline_path_data_core(&path, options.clone())?;
        build_centerline_output(&d, &options.emit, stroke_width, &options, ctx)
    } else if let Some(input) = input {
        let text = read_file(&input)?;
        if options.svg_paths == "all" {
            centerline_svg_text_core(&text, options.clone())?
        } else {
            let d = find_d_attributes(&text).first().map(|m| m.3.clone()).unwrap_or(text.trim().to_string());
            let (center_d, stroke_width, ctx) = centerline_path_data_core(&d, options.clone())?;
            build_centerline_output(&center_d, &options.emit, stroke_width, &options, ctx)
        }
    } else {
        return Err("Provide exactly one of --path or --input".into());
    };
    write_file_or_return(result, output)
}

fn run_info_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut input = None;
    let mut output = None;
    let mut compact = false;
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--compact" => compact = true,
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let input = input.ok_or_else(|| SvgoError("--input is required".to_string()))?;
    let info = get_svg_info_value(&input);
    if let Some(err) = info.get("error").and_then(Value::as_str) {
        return Err(err.to_string().into());
    }
    let text = if compact { info.to_string() } else { serde_json::to_string_pretty(&info).unwrap() };
    write_file_or_return(text, output)
}

fn format_validation_result(result: &Value) -> String {
    let mut lines = vec![if result.get("valid").and_then(Value::as_bool).unwrap_or(false) { "valid" } else { "invalid" }.to_string()];
    if let Some(issues) = result.get("issues").and_then(Value::as_array) {
        for issue in issues {
            let level = issue.get("level").and_then(Value::as_str).unwrap_or("issue");
            let reason = issue.get("reason").and_then(Value::as_str).unwrap_or("");
            lines.push(format!("{}: {}", level, reason));
        }
    }
    lines.join("\n")
}

fn run_validate_cli(args: &[String]) -> Result<(String, bool, Option<String>)> {
    let mut cursor = ArgCursor::new(args);
    let mut input = None;
    let mut output = None;
    let mut strict = false;
    let mut emit_json = false;
    let mut compact = false;
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--strict" => strict = true,
            "--json" => emit_json = true,
            "--compact" => compact = true,
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let input = input.ok_or_else(|| SvgoError("--input is required".to_string()))?;
    let result = validate_svg_value(&input, strict);
    let valid = result.get("valid").and_then(Value::as_bool).unwrap_or(false);
    let text = if emit_json {
        if compact { result.to_string() } else { serde_json::to_string_pretty(&result).unwrap() }
    } else {
        format_validation_result(&result)
    };
    if let Some(output) = output.clone() {
        fs::write(&output, if text.ends_with('\n') { text.clone() } else { format!("{}\n", text) }).map_err(|e| SvgoError(e.to_string()))?;
        Ok((String::new(), valid, None))
    } else {
        Ok((text, valid, None))
    }
}

fn run_measure_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut path = None;
    let mut input = None;
    let mut output = None;
    let mut at = None;
    let mut decimals = None;
    let mut error = 0.01;
    let mut compact = false;
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--path" => path = Some(cursor.value("--path")?),
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--at" => at = Some(cursor.value("--at")?.parse().map_err(|_| SvgoError("--at must be a number".to_string()))?),
            "--decimals" => decimals = Some(cursor.value("--decimals")?.parse().map_err(|_| SvgoError("--decimals must be an integer".to_string()))?),
            "--error" => error = cursor.value("--error")?.parse().map_err(|_| SvgoError("--error must be a number".to_string()))?,
            "--compact" => compact = true,
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let mut result = if let Some(ref path) = path {
        path_metrics_value(&path, decimals, error)?
    } else if let Some(input) = input {
        let text = read_file(&input)?;
        if contains_svg_markup(&text) {
            svg_metrics_value(&text, decimals, error)
        } else {
            path_metrics_value(text.trim(), decimals, error)?
        }
    } else {
        return Err("Provide --path or --input".into());
    };
    if let Some(distance) = at {
        let d = if let Some(path) = result.get("paths").and_then(Value::as_array).and_then(|paths| {
            if paths.len() == 1 { paths[0].get("d").and_then(Value::as_str) } else { None }
        }) {
            path.to_string()
        } else if let Some(ref p) = path {
            p.clone()
        } else {
            return Err("--at requires raw path input or an SVG with exactly one measurable path".into());
        };
        let point_text = point_at_length_json(&d, distance, Some(error)).map_err(|e| SvgoError(e.to_string()))?;
        let mut point: Value = serde_json::from_str(&point_text)
            .map_err(|e| SvgoError(e.to_string()))?;
        if let Some(decimals) = decimals {
            if let Some(map) = point.as_object_mut() {
                for value in map.values_mut() {
                    if let Some(number) = value.as_f64() {
                        *value = json!(round_to(number, decimals));
                    }
                }
            }
        }
        if let Some(map) = result.as_object_mut() {
            map.insert("point".to_string(), point);
        }
    }
    let text = if compact { result.to_string() } else { serde_json::to_string_pretty(&result).unwrap() };
    write_file_or_return(text, output)
}

fn run_sanitize_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut input = None;
    let mut output = None;
    let mut precision = None;
    let mut remove_external_refs = false;
    let mut allow_data_images = true;
    let mut remove_styles = false;
    let mut remove_raster_images = false;
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--precision" => precision = Some(cursor.value("--precision")?.parse().map_err(|_| SvgoError("--precision must be an integer".to_string()))?),
            "--remove-external-refs" => remove_external_refs = true,
            "--disallow-data-images" => allow_data_images = false,
            "--remove-styles" => remove_styles = true,
            "--remove-raster-images" => remove_raster_images = true,
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let input = input.ok_or_else(|| SvgoError("--input is required".to_string()))?;
    let result = sanitize_svg_core(&read_file(&input)?, precision, remove_external_refs, allow_data_images, remove_styles, remove_raster_images)?;
    write_file_or_return(result, output)
}

fn run_viewbox_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut input = None;
    let mut output = None;
    let mut set = None;
    let mut fit_content = false;
    let mut padding = 0.0;
    let mut width = None;
    let mut height = None;
    let mut remove_dimensions = false;
    let mut precision = None;
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--set" => set = Some(cursor.value("--set")?),
            "--fit-content" => fit_content = true,
            "--padding" => padding = cursor.value("--padding")?.parse().map_err(|_| SvgoError("--padding must be a number".to_string()))?,
            "--width" => width = Some(cursor.value("--width")?),
            "--height" => height = Some(cursor.value("--height")?),
            "--remove-dimensions" => remove_dimensions = true,
            "--precision" => precision = Some(cursor.value("--precision")?.parse().map_err(|_| SvgoError("--precision must be an integer".to_string()))?),
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let input = input.ok_or_else(|| SvgoError("--input is required".to_string()))?;
    let mut text = read_file(&input)?;
    if fit_content {
        text = fit_viewbox_svg(&text, padding, precision, remove_dimensions).map_err(|e| SvgoError(e.to_string()))?;
    } else if let Some(ref set) = set {
        text = set_viewbox_svg_core(&text, &set, precision, remove_dimensions)?;
    } else if remove_dimensions {
        return Err("--remove-dimensions requires --set or --fit-content".into());
    }
    if width.is_some() || height.is_some() {
        text = resize_svg(&text, width.as_deref(), height.as_deref()).map_err(|e| SvgoError(e.to_string()))?;
    }
    if !fit_content && width.is_none() && height.is_none() && set.is_none() {
        return Err("Provide --set, --fit-content, --width, or --height".into());
    }
    write_file_or_return(text, output)
}

fn run_convert_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut input = None;
    let mut output = None;
    let mut precision = None;
    let mut plain = false;
    let mut shapes_to_paths = false;
    let mut flatten_transforms = false;
    let mut flatten_groups = false;
    let mut inline_styles = false;
    let mut sanitize = false;
    let mut all = false;
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--precision" => precision = Some(cursor.value("--precision")?.parse().map_err(|_| SvgoError("--precision must be an integer".to_string()))?),
            "--to-plain" => plain = true,
            "--shapes-to-paths" => shapes_to_paths = true,
            "--flatten-transforms" => flatten_transforms = true,
            "--flatten-groups" => flatten_groups = true,
            "--inline-styles" => inline_styles = true,
            "--sanitize" => sanitize = true,
            "--all" => all = true,
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let input = input.ok_or_else(|| SvgoError("--input is required".to_string()))?;
    let explicit = plain || shapes_to_paths || flatten_transforms || flatten_groups || inline_styles || sanitize || all;
    let mut text = read_file(&input)?;
    if sanitize || all {
        text = sanitize_svg_core(&text, precision, false, true, false, false)?;
    }
    if inline_styles || all {
        text = inline_styles_svg(&text, precision, true).map_err(|e| SvgoError(e.to_string()))?;
    }
    plain = plain || all;
    shapes_to_paths = shapes_to_paths || all || !explicit;
    flatten_transforms = flatten_transforms || all;
    flatten_groups = flatten_groups || all;
    let result = if plain && !(shapes_to_paths || flatten_transforms || flatten_groups) {
        to_plain_svg_core(&text, precision)?
    } else if shapes_to_paths && !(plain || flatten_transforms || flatten_groups) {
        convert_shapes_svg(&text, precision).map_err(|e| SvgoError(e.to_string()))?
    } else {
        flatten_svg(&text, precision, flatten_transforms, flatten_groups, shapes_to_paths, plain).map_err(|e| SvgoError(e.to_string()))?
    };
    write_file_or_return(result, output)
}

fn help_text() -> String {
    "svgo <path|opt|trace|trace2|center|info|validate|measure|sanitize|viewbox|convert|plugins> [options]\nsvgo --version".to_string()
}

fn version_text() -> String {
    format!("svgo {}", env!("CARGO_PKG_VERSION"))
}

fn cli_run_internal(args: Vec<String>) -> (i32, String, String) {
    if args.is_empty() {
        return (0, help_text(), String::new());
    }
    let command = &args[0];
    if command == "--version" || command == "-V" || command == "version" {
        return (0, version_text(), String::new());
    }
    let rest = &args[1..];
    let result = match command.as_str() {
        "path" | "p" => run_path_cli(rest).map(|out| (0, out)),
        "opt" | "o" => run_opt_cli(rest).map(|out| (0, out)),
        "trace" | "t" => run_trace_cli(rest).map(|out| (0, out)),
        "trace2" | "t2" => run_trace2_cli(rest).map(|out| (0, out)),
        "center" | "c" => run_center_cli(rest).map(|out| (0, out)),
        "info" | "i" => run_info_cli(rest).map(|out| (0, out)),
        "validate" | "v" => match run_validate_cli(rest) {
            Ok((out, valid, _)) => Ok((if valid { 0 } else { 1 }, out)),
            Err(err) => Err(err),
        },
        "measure" | "m" => run_measure_cli(rest).map(|out| (0, out)),
        "sanitize" | "s" => run_sanitize_cli(rest).map(|out| (0, out)),
        "viewbox" | "b" => run_viewbox_cli(rest).map(|out| (0, out)),
        "convert" | "x" => run_convert_cli(rest).map(|out| (0, out)),
        "plugins" | "l" => Ok((0, plugin_list_text())),
        _ => return (2, String::new(), format!("invalid choice: '{}'", command)),
    };
    match result {
        Ok((code, stdout)) => (code, stdout, String::new()),
        Err(err) => (1, String::new(), err.to_string()),
    }
}

pub fn cli_main(args: Vec<String>) -> i32 {
    let (code, stdout, stderr) = cli_run_internal(args);
    if !stdout.is_empty() {
        println!("{}", stdout);
    }
    if !stderr.is_empty() {
        eprintln!("{}", stderr);
    }
    code
}

#[pyfunction]
fn cli_run(args: Vec<String>) -> (i32, String, String) {
    cli_run_internal(args)
}

#[pymodule]
fn _svgo(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    m.add_class::<PyPathData>()?;
    m.add_function(wrap_pyfunction!(parse_path_json, m)?)?;
    m.add_function(wrap_pyfunction!(path_to_absolute, m)?)?;
    m.add_function(wrap_pyfunction!(path_to_relative, m)?)?;
    m.add_function(wrap_pyfunction!(transform_path, m)?)?;
    m.add_function(wrap_pyfunction!(path_to_cubics, m)?)?;
    m.add_function(wrap_pyfunction!(translate_2d, m)?)?;
    m.add_function(wrap_pyfunction!(scale_2d, m)?)?;
    m.add_function(wrap_pyfunction!(rotate_2d, m)?)?;
    m.add_function(wrap_pyfunction!(multiply_matrices, m)?)?;
    m.add_function(wrap_pyfunction!(transform_2d, m)?)?;
    m.add_function(wrap_pyfunction!(matrix_to_3x3, m)?)?;
    m.add_function(wrap_pyfunction!(identity, m)?)?;
    m.add_function(wrap_pyfunction!(rect_to_path, m)?)?;
    m.add_function(wrap_pyfunction!(line_to_path, m)?)?;
    m.add_function(wrap_pyfunction!(circle_to_path, m)?)?;
    m.add_function(wrap_pyfunction!(ellipse_to_path, m)?)?;
    m.add_function(wrap_pyfunction!(polyline_to_path, m)?)?;
    m.add_function(wrap_pyfunction!(polygon_to_path, m)?)?;
    m.add_function(wrap_pyfunction!(path_length, m)?)?;
    m.add_function(wrap_pyfunction!(path_bbox_json, m)?)?;
    m.add_function(wrap_pyfunction!(path_metrics_json, m)?)?;
    m.add_function(wrap_pyfunction!(point_at_length_json, m)?)?;
    m.add_function(wrap_pyfunction!(svg_metrics_json, m)?)?;
    m.add_function(wrap_pyfunction!(builtin_plugins_json, m)?)?;
    m.add_function(wrap_pyfunction!(optimize_svg, m)?)?;
    m.add_function(wrap_pyfunction!(validate_svg_json, m)?)?;
    m.add_function(wrap_pyfunction!(get_svg_info_json, m)?)?;
    m.add_function(wrap_pyfunction!(to_plain_svg, m)?)?;
    m.add_function(wrap_pyfunction!(sanitize_svg, m)?)?;
    m.add_function(wrap_pyfunction!(inline_styles_svg, m)?)?;
    m.add_function(wrap_pyfunction!(convert_shapes_svg, m)?)?;
    m.add_function(wrap_pyfunction!(flatten_svg, m)?)?;
    m.add_function(wrap_pyfunction!(set_viewbox_svg, m)?)?;
    m.add_function(wrap_pyfunction!(fit_viewbox_svg, m)?)?;
    m.add_function(wrap_pyfunction!(resize_svg, m)?)?;
    m.add_function(wrap_pyfunction!(trace_image, m)?)?;
    m.add_function(wrap_pyfunction!(trace_png, m)?)?;
    m.add_function(wrap_pyfunction!(trace_image_components_json, m)?)?;
    m.add_function(wrap_pyfunction!(trace_png_components_json, m)?)?;
    m.add_function(wrap_pyfunction!(trace_image_vtracer, m)?)?;
    m.add_function(wrap_pyfunction!(centerline_path_data_json, m)?)?;
    m.add_function(wrap_pyfunction!(centerline_svg_text, m)?)?;
    m.add_function(wrap_pyfunction!(cli_run, m)?)?;
    Ok(())
}
