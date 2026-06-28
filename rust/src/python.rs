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
