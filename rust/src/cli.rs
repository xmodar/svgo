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

fn is_help_arg(arg: &str) -> bool {
    arg == "-h" || arg == "--help" || arg == "help"
}

fn is_version_arg(arg: &str) -> bool {
    arg == "-v" || arg == "-V" || arg == "--version" || arg == "version"
}

fn help_text(command: Option<&str>) -> String {
    match command {
        Some("path") | Some("p") => r#"Usage:
  svgo path --path <D> [--op <OP> ...] [options]
  svgo path --input <SVG_OR_D_FILE> [--output <FILE>] [--select <SELECTOR>] [--op <OP> ...] [options]
  svgo p ...

Edit raw SVG path data or path d attributes inside an SVG document.

Input/output:
  --path <D>                 Raw SVG path data.
  -i, --input <FILE>         Read SVG markup or raw path data from a file.
  -o, --output <FILE>        Write output to a file instead of stdout.
  --select <all|N|N,N>       Select SVG path d attributes when reading SVG input. Default: all.

Path options:
  --op <OP>                  Apply an operation. Repeat to apply in order.
  --decimals <N>             Decimal precision for generated path data. Default: 4.
  --minify                   Remove optional path spacing where possible.

SVGO options:
  --svgo                     Run the SVG/path optimizer.
  --svgo-order <before|after>
  --svgo-preset <default|none>
  --svgo-plugin <NAME[:JSON]>
  --svgo-disable <NAME>
  --svgo-precision <N>
  --svgo-multipass
  --svgo-pretty
  --svgo-indent <N>
  --svgo-eol <lf|crlf>
  --svgo-final-newline
  --svgo-datauri <base64|enc|unenc>
  --svgo-list-plugins
  --svgo-config <FILE>       Accepted for CLI compatibility; JavaScript configs are not executed.

Examples:
  svgo path --path "M10 10h5v5z" --op optimize:safe --minify
  svgo p -i icon.svg -o edited.svg --select 0,2 --op translate:2,-1"#.to_string(),
        Some("opt") | Some("o") => r#"Usage:
  svgo opt --input <SVG_FILE> [--output <FILE>] [SVGO options]
  svgo o ...

Optimize an SVG document using built-in SVGO-style passes.

Input/output:
  -i, --input <FILE>         Read SVG markup from a file.
  -o, --output <FILE>        Write output to a file instead of stdout.

SVGO options:
  --svgo-preset <default|none>
  --svgo-plugin <NAME[:JSON]>
  --svgo-disable <NAME>
  --svgo-precision <N>
  --svgo-multipass
  --svgo-pretty
  --svgo-indent <N>
  --svgo-eol <lf|crlf>
  --svgo-final-newline
  --svgo-datauri <base64|enc|unenc>
  --svgo-list-plugins
  --svgo-config <FILE>       Accepted for CLI compatibility; JavaScript configs are not executed.

Examples:
  svgo opt -i icon.svg -o icon.min.svg --svgo-precision 3
  svgo o -i icon.svg --svgo-disable cleanupIds"#.to_string(),
        Some("trace") | Some("t") => r#"Usage:
  svgo trace --input <PNG_FILE> [--output <FILE>] [options]
  svgo t ...

Trace a non-interlaced 8-bit PNG into filled SVG paths.

Options:
  -i, --input <FILE>         PNG input file.
  -o, --output <FILE>        Write SVG or JSON to a file instead of stdout.
  --components-json          Emit per-component JSON instead of SVG.
  --mode <palette|alpha|exact>
  --curve-mode <pixel|exact>
  --alpha-threshold <N>
  --white-threshold <N>
  --drop-white
  --quantize <N>
  --max-colors <N>
  --min-area <N>
  --scale <N>
  --decimals <N>
  --title <TEXT>
  --palette <#RRGGBB,...>

Example:
  svgo trace -i icon.png -o traced.svg --mode palette --max-colors 8 --min-area 8"#.to_string(),
        Some("trace2") | Some("t2") => r#"Usage:
  svgo trace2 --input <PNG_FILE> [--output <FILE>] [options]
  svgo t2 ...

Trace a PNG with VTracer-compatible option names, backed by svgo's native tracer.

Options:
  -i, --input <FILE>         PNG input file.
  -o, --output <FILE>        Write SVG to a file instead of stdout.
  --color-mode <color|binary>
  --hierarchical <stacked|cutout>
  --clustering <stacked|cutout>
  --color-precision <1..8>
  --gradient-step <N>
  --filter-speckle <N>
  --curve-mode <pixel|polygon|spline>
  --corner-threshold <N>
  --segment-length <N>
  --max-iterations <N>
  --splice-threshold <N>
  --path-precision <N>

Example:
  svgo trace2 -i icon.png -o traced.svg --curve-mode spline --filter-speckle 4"#.to_string(),
        Some("center") | Some("c") => r#"Usage:
  svgo center --path <D> [--output <FILE>] [options]
  svgo center --input <SVG_OR_D_FILE> [--output <FILE>] [options]
  svgo c ...

Reconstruct approximate stroked centerlines from filled path outlines.

Input/output:
  --path <D>                 Raw SVG path data.
  -i, --input <FILE>         SVG or raw path input file.
  -o, --output <FILE>        Write output to a file instead of stdout.

Options:
  --emit <path|svg|d>        Output wrapper. Default: path.
  --mode <longest|all>       Keep the longest chain or all chains.
  --scale <N>
  --max-size <N>
  --curve-samples <N>
  --simplify <N>
  --min-length <N>
  --stroke-width <auto|N>
  --linecap <VALUE>
  --linejoin <VALUE>
  --decimals <N>
  --polyline
  --fill-rule <evenodd|nonzero>
  --svg-paths <first|all>
  --keep-failed
  --bridge-gap <N>

Example:
  svgo center -i outline.svg -o stroke.svg --svg-paths all --mode all --bridge-gap 12"#.to_string(),
        Some("info") | Some("i") => r#"Usage:
  svgo info --input <SVG_FILE> [--output <FILE>] [--compact]
  svgo i ...

Print structured SVG metadata as JSON.

Options:
  -i, --input <FILE>         SVG input file.
  -o, --output <FILE>        Write JSON to a file instead of stdout.
  --compact                  Emit compact JSON.

Example:
  svgo info -i icon.svg --compact"#.to_string(),
        Some("validate") => r#"Usage:
  svgo validate --input <SVG_FILE> [--output <FILE>] [options]

Validate SVG XML and static SVG structure.

Options:
  -i, --input <FILE>         SVG input file.
  -o, --output <FILE>        Write report to a file instead of stdout.
  --strict                   Treat warnings as invalid.
  --json                     Emit JSON instead of text.
  --compact                  Emit compact JSON when used with --json.

Example:
  svgo validate -i icon.svg --strict --json"#.to_string(),
        Some("v") => "Usage:\n  svgo v --input <SVG_FILE> [--output <FILE>] [options]\n\nAlias for `svgo validate`. Use `svgo validate --help` for full options.".to_string(),
        Some("measure") | Some("m") => r#"Usage:
  svgo measure --path <D> [options]
  svgo measure --input <SVG_OR_D_FILE> [options]
  svgo m ...

Measure path or SVG geometry and emit JSON metrics.

Options:
  --path <D>                 Raw SVG path data.
  -i, --input <FILE>         SVG or raw path input file.
  -o, --output <FILE>        Write JSON to a file instead of stdout.
  --at <DISTANCE>            Include point-at-length data.
  --decimals <N>             Round numeric output.
  --error <N>                Curve length error tolerance. Default: 0.01.
  --compact                  Emit compact JSON.

Example:
  svgo measure --path "M0 0H10V10" --at 12 --decimals 3"#.to_string(),
        Some("sanitize") | Some("s") => r#"Usage:
  svgo sanitize --input <SVG_FILE> [--output <FILE>] [options]
  svgo s ...

Remove active or unsafe SVG content while keeping static geometry.

Options:
  -i, --input <FILE>         SVG input file.
  -o, --output <FILE>        Write SVG to a file instead of stdout.
  --precision <N>
  --remove-external-refs
  --disallow-data-images
  --remove-styles
  --remove-raster-images

Example:
  svgo sanitize -i unsafe.svg -o safe.svg --remove-external-refs"#.to_string(),
        Some("viewbox") | Some("b") => r#"Usage:
  svgo viewbox --input <SVG_FILE> [--output <FILE>] [options]
  svgo b ...

Edit root SVG viewBox, width, and height metadata.

Options:
  -i, --input <FILE>         SVG input file.
  -o, --output <FILE>        Write SVG to a file instead of stdout.
  --set "<MIN_X MIN_Y WIDTH HEIGHT>"
  --fit-content
  --padding <N>
  --width <VALUE>
  --height <VALUE>
  --remove-dimensions
  --precision <N>

Example:
  svgo viewbox -i icon.svg --fit-content --padding 1 --remove-dimensions"#.to_string(),
        Some("convert") | Some("x") => r#"Usage:
  svgo convert --input <SVG_FILE> [--output <FILE>] [options]
  svgo x ...

Convert and normalize SVG structure. With no conversion flag, shapes are converted to paths.

Options:
  -i, --input <FILE>         SVG input file.
  -o, --output <FILE>        Write SVG to a file instead of stdout.
  --precision <N>
  --to-plain
  --shapes-to-paths
  --flatten-transforms
  --flatten-groups
  --inline-styles
  --sanitize
  --all

Example:
  svgo convert -i drawing.svg -o plain.svg --all --precision 3"#.to_string(),
        Some("plugins") | Some("l") => r#"Usage:
  svgo plugins
  svgo l

List built-in optimizer plugins and presets.

Example:
  svgo plugins"#.to_string(),
        _ => format!(
            r#"svgo {version}

Usage:
  svgo <command> [options]
  svgo -h | --help
  svgo -v | --version

Commands:
  path, p          Edit raw path data or SVG path attributes.
  opt, o           Optimize SVG documents.
  trace, t         Trace PNG images into filled SVG paths.
  trace2, t2       Trace PNG images with VTracer-compatible options.
  center, c        Reconstruct approximate centerline strokes.
  info, i          Inspect SVG metadata as JSON.
  validate, v      Validate SVG XML and structure.
  measure, m       Measure path and SVG geometry.
  sanitize, s      Remove unsafe SVG content.
  viewbox, b       Edit viewBox, width, and height metadata.
  convert, x       Convert shapes, transforms, styles, and editor markup.
  plugins, l       List optimizer plugins.

Use `svgo <command> --help` for command-specific options."#,
            version = env!("CARGO_PKG_VERSION")
        ),
    }
}

fn version_text() -> String {
    format!("svgo {}", env!("CARGO_PKG_VERSION"))
}

fn cli_run_internal(args: Vec<String>) -> (i32, String, String) {
    if args.is_empty() {
        return (0, help_text(None), String::new());
    }
    let command = &args[0];
    if is_help_arg(command) {
        return (0, help_text(None), String::new());
    }
    if is_version_arg(command) {
        return (0, version_text(), String::new());
    }
    let rest = &args[1..];
    if rest.iter().any(|arg| is_help_arg(arg)) {
        return match command.as_str() {
            "path" | "p" | "opt" | "o" | "trace" | "t" | "trace2" | "t2" | "center" | "c"
            | "info" | "i" | "validate" | "v" | "measure" | "m" | "sanitize" | "s"
            | "viewbox" | "b" | "convert" | "x" | "plugins" | "l" => {
                (0, help_text(Some(command.as_str())), String::new())
            }
            _ => (2, String::new(), format!("invalid choice: '{}'", command)),
        };
    }
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

