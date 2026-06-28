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

