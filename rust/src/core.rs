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
