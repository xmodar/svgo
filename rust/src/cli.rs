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

fn recipe_value<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| value.get(*key))
}

fn recipe_string(value: &Value, keys: &[&str]) -> Option<String> {
    recipe_value(value, keys).and_then(|item| match item {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    })
}

fn recipe_bool(value: &Value, keys: &[&str], default: bool) -> bool {
    recipe_value(value, keys)
        .and_then(|item| match item {
            Value::Bool(flag) => Some(*flag),
            Value::String(text) => match text.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(default)
}

fn recipe_usize(value: &Value, keys: &[&str]) -> Result<Option<usize>> {
    let Some(item) = recipe_value(value, keys) else {
        return Ok(None);
    };
    match item {
        Value::Number(number) => number
            .as_u64()
            .map(|n| Some(n as usize))
            .ok_or_else(|| SvgoError(format!("{} must be a non-negative integer", keys[0]))),
        Value::String(text) => text
            .parse::<usize>()
            .map(Some)
            .map_err(|_| SvgoError(format!("{} must be a non-negative integer", keys[0]))),
        _ => Err(SvgoError(format!("{} must be a non-negative integer", keys[0]))),
    }
}

fn recipe_f64(value: &Value, keys: &[&str]) -> Result<Option<f64>> {
    let Some(item) = recipe_value(value, keys) else {
        return Ok(None);
    };
    match item {
        Value::Number(number) => number
            .as_f64()
            .map(Some)
            .ok_or_else(|| SvgoError(format!("{} must be a number", keys[0]))),
        Value::String(text) => text
            .parse::<f64>()
            .map(Some)
            .map_err(|_| SvgoError(format!("{} must be a number", keys[0]))),
        _ => Err(SvgoError(format!("{} must be a number", keys[0]))),
    }
}

fn recipe_string_list(value: &Value, keys: &[&str]) -> Result<Vec<String>> {
    let Some(item) = recipe_value(value, keys) else {
        return Ok(Vec::new());
    };
    match item {
        Value::String(text) => Ok(vec![text.clone()]),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| SvgoError(format!("{} entries must be strings", keys[0])))
            })
            .collect(),
        _ => Err(SvgoError(format!("{} must be a string or string array", keys[0]))),
    }
}

fn recipe_plugin_specs(value: &Value, keys: &[&str]) -> Result<Vec<PluginSpecCore>> {
    let Some(item) = recipe_value(value, keys) else {
        return Ok(Vec::new());
    };
    let items = item
        .as_array()
        .ok_or_else(|| SvgoError(format!("{} must be an array", keys[0])))?;
    let mut plugins = Vec::new();
    for item in items {
        if let Some(spec) = item.as_str() {
            plugins.push(parse_plugin_spec_cli(spec)?);
        } else if let Some(object) = item.as_object() {
            let name = object
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| SvgoError("recipe plugin objects require a name".to_string()))?;
            let params = object.get("params").cloned().unwrap_or_else(|| json!({}));
            if !params.is_object() {
                return Err(SvgoError(format!("Plugin params for {} must be a JSON object", name)));
            }
            plugins.push(PluginSpecCore { name: name.to_string(), params });
        } else {
            return Err(SvgoError("recipe plugins must be strings or objects".to_string()));
        }
    }
    Ok(plugins)
}

fn recipe_opt_options(step: &Value) -> Result<OptimizeOptionsCore> {
    let mut options = OptimizeOptionsCore::default();
    if let Some(preset) = recipe_string(step, &["preset", "svgoPreset", "svgo_preset"]) {
        options.preset = preset;
    }
    if let Some(precision) = recipe_usize(step, &["precision", "svgoPrecision", "svgo_precision", "floatPrecision", "float_precision"])? {
        options.float_precision = Some(precision.min(20));
    }
    options.plugins = recipe_plugin_specs(step, &["plugins", "svgoPlugins", "svgo_plugins"])?;
    options.disabled = recipe_string_list(step, &["disabled", "disable", "svgoDisabled", "svgo_disabled"])?;
    options.multipass = recipe_bool(step, &["multipass", "svgoMultipass", "svgo_multipass"], false);
    options.pretty = recipe_bool(step, &["pretty", "svgoPretty", "svgo_pretty"], false);
    if let Some(indent) = recipe_usize(step, &["indent", "svgoIndent", "svgo_indent"])? {
        options.indent = indent;
    }
    options.eol = recipe_string(step, &["eol", "svgoEol", "svgo_eol"]);
    options.final_newline = recipe_bool(step, &["finalNewline", "final_newline", "svgoFinalNewline", "svgo_final_newline"], false);
    options.datauri = recipe_string(step, &["datauri", "dataUri", "data_uri", "svgoDatauri", "svgo_datauri"]);
    Ok(options)
}

fn recipe_trace_options(step: &Value) -> Result<TraceOptionsCore> {
    let mut options = TraceOptionsCore::default();
    if let Some(mode) = recipe_string(step, &["mode"]) {
        options.mode = mode;
    }
    if let Some(curve_mode) = recipe_string(step, &["curveMode", "curve_mode"]) {
        options.curve_mode = curve_mode;
    }
    if let Some(value) = recipe_usize(step, &["alphaThreshold", "alpha_threshold"])? {
        options.alpha_threshold = value.min(255) as u8;
    }
    if let Some(value) = recipe_usize(step, &["whiteThreshold", "white_threshold"])? {
        options.white_threshold = value.min(255) as u8;
    }
    options.drop_white = recipe_bool(step, &["dropWhite", "drop_white"], false);
    if let Some(value) = recipe_usize(step, &["quantize"])? {
        options.quantize = value.max(1).min(255) as u8;
    }
    if let Some(value) = recipe_usize(step, &["maxColors", "max_colors"])? {
        options.max_colors = value;
    }
    if let Some(value) = recipe_usize(step, &["minArea", "min_area"])? {
        options.min_area = value;
    }
    if let Some(value) = recipe_f64(step, &["scale"])? {
        options.scale = value;
    }
    if let Some(value) = recipe_usize(step, &["decimals", "precision"])? {
        options.decimals = value;
    }
    options.title = recipe_string(step, &["title"]);
    options.palette = recipe_string_list(step, &["palette"])?;
    Ok(options)
}

fn recipe_vtracer_options(step: &Value) -> Result<VTracerOptionsCore> {
    let mut options = VTracerOptionsCore::default();
    if let Some(value) = recipe_string(step, &["colorMode", "color_mode"]) {
        options.color_mode = value;
    }
    if let Some(value) = recipe_string(step, &["hierarchical", "clustering"]) {
        options.hierarchical = value;
    }
    if let Some(value) = recipe_usize(step, &["colorPrecision", "color_precision"])? {
        options.color_precision = value;
    }
    if let Some(value) = recipe_usize(step, &["gradientStep", "gradient_step"])? {
        options.gradient_step = value;
    }
    if let Some(value) = recipe_usize(step, &["filterSpeckle", "filter_speckle"])? {
        options.filter_speckle = value;
    }
    if let Some(value) = recipe_string(step, &["curveMode", "curve_mode"]) {
        options.curve_mode = value;
    }
    if let Some(value) = recipe_usize(step, &["cornerThreshold", "corner_threshold"])? {
        options.corner_threshold = value;
    }
    if let Some(value) = recipe_f64(step, &["segmentLength", "segment_length"])? {
        options.segment_length = value;
    }
    if let Some(value) = recipe_usize(step, &["maxIterations", "max_iterations"])? {
        options.max_iterations = value;
    }
    if let Some(value) = recipe_usize(step, &["spliceThreshold", "splice_threshold"])? {
        options.splice_threshold = value;
    }
    if let Some(value) = recipe_usize(step, &["pathPrecision", "path_precision", "decimals", "precision"])? {
        options.path_precision = value;
    }
    parse_vtracer_options(Some(&serde_json::to_string(&options).unwrap()))?;
    Ok(options)
}

fn recipe_center_options(step: &Value) -> Result<CenterlineOptionsCore> {
    let mut options = CenterlineOptionsCore::default();
    if let Some(value) = recipe_string(step, &["emit"]) {
        options.emit = value;
    }
    if let Some(value) = recipe_string(step, &["mode"]) {
        options.mode = value;
    }
    if let Some(value) = recipe_f64(step, &["scale"])? {
        options.scale = value;
    }
    if let Some(value) = recipe_usize(step, &["maxSize", "max_size"])? {
        options.max_size = value;
    }
    if let Some(value) = recipe_usize(step, &["curveSamples", "curve_samples"])? {
        options.curve_samples = value;
    }
    if let Some(value) = recipe_f64(step, &["simplify"])? {
        options.simplify = value;
    }
    if let Some(value) = recipe_f64(step, &["minLength", "min_length"])? {
        options.min_length = value;
    }
    if let Some(value) = recipe_string(step, &["strokeWidth", "stroke_width"]) {
        options.stroke_width = value;
    }
    if let Some(value) = recipe_string(step, &["linecap", "lineCap", "line_cap"]) {
        options.linecap = value;
    }
    if let Some(value) = recipe_string(step, &["linejoin", "lineJoin", "line_join"]) {
        options.linejoin = value;
    }
    if let Some(value) = recipe_usize(step, &["decimals", "precision"])? {
        options.decimals = value;
    }
    options.polyline = recipe_bool(step, &["polyline"], false);
    if let Some(value) = recipe_string(step, &["fillRule", "fill_rule"]) {
        options.fill_rule = value;
    }
    if let Some(value) = recipe_string(step, &["svgPaths", "svg_paths"]) {
        options.svg_paths = value;
    }
    options.keep_failed = recipe_bool(step, &["keepFailed", "keep_failed"], false);
    if let Some(value) = recipe_f64(step, &["bridgeGap", "bridge_gap"])? {
        options.bridge_gap = value;
    }
    Ok(options)
}

fn recipe_current_text(current: &Option<String>, input_path: &Path) -> Result<String> {
    if let Some(text) = current {
        Ok(text.clone())
    } else {
        fs::read_to_string(input_path).map_err(|e| SvgoError(e.to_string()))
    }
}

fn recipe_step_command(step: &Value) -> Result<String> {
    recipe_string(step, &["command", "cmd", "use", "uses", "op"])
        .map(|command| command.to_ascii_lowercase())
        .ok_or_else(|| SvgoError("recipe step requires a command".to_string()))
}

fn apply_recipe_step(current: Option<String>, input_path: &Path, step: &Value) -> Result<(Option<String>, Value)> {
    let command = recipe_step_command(step)?;
    let mut report = json!({"command": command.clone()});
    let next = match command.as_str() {
        "trace" | "t" => {
            let image = read_png(input_path)?;
            let options = recipe_trace_options(step)?;
            if recipe_bool(step, &["componentsJson", "components_json"], false) {
                Some(trace_components_value(&image, options)?.to_string())
            } else {
                Some(trace_image_core(&image, options)?)
            }
        }
        "trace2" | "t2" => {
            let image = match read_png(input_path) {
                Ok(image) => image,
                Err(err) => {
                    if fs::read(input_path).is_ok_and(|data| data.starts_with(PNG_SIGNATURE)) {
                        return Ok((Some(r#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#.to_string()), report));
                    }
                    return Err(err);
                }
            };
            let options = recipe_vtracer_options(step)?;
            let trace_options = TraceOptionsCore {
                mode: if options.color_mode == "binary" { "alpha".to_string() } else { "palette".to_string() },
                max_colors: 1usize << options.color_precision.min(4),
                quantize: (256usize / (1usize << options.color_precision.min(8))).max(1) as u8,
                min_area: options.filter_speckle.max(1),
                decimals: options.path_precision,
                curve_mode: "pixel".to_string(),
                ..Default::default()
            };
            Some(trace_image_core(&image, trace_options)?)
        }
        "sanitize" | "s" => {
            let text = recipe_current_text(&current, input_path)?;
            let precision = recipe_usize(step, &["precision"])?;
            let remove_external_refs = recipe_bool(step, &["removeExternalRefs", "remove_external_refs"], false);
            let allow_data_images = !recipe_bool(step, &["disallowDataImages", "disallow_data_images"], false);
            let remove_styles = recipe_bool(step, &["removeStyles", "remove_styles"], false);
            let remove_raster_images = recipe_bool(step, &["removeRasterImages", "remove_raster_images"], false);
            Some(sanitize_svg_core(&text, precision, remove_external_refs, allow_data_images, remove_styles, remove_raster_images)?)
        }
        "convert" | "x" => {
            let mut text = recipe_current_text(&current, input_path)?;
            let precision = recipe_usize(step, &["precision"])?;
            let mut plain = recipe_bool(step, &["toPlain", "to_plain", "plain"], false);
            let mut shapes_to_paths = recipe_bool(step, &["shapesToPaths", "shapes_to_paths"], false);
            let mut flatten_transforms = recipe_bool(step, &["flattenTransforms", "flatten_transforms"], false);
            let mut flatten_groups = recipe_bool(step, &["flattenGroups", "flatten_groups"], false);
            let inline_styles = recipe_bool(step, &["inlineStyles", "inline_styles"], false);
            let sanitize = recipe_bool(step, &["sanitize"], false);
            let all = recipe_bool(step, &["all"], false);
            let explicit = plain || shapes_to_paths || flatten_transforms || flatten_groups || inline_styles || sanitize || all;
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
            if plain && !(shapes_to_paths || flatten_transforms || flatten_groups) {
                Some(to_plain_svg_core(&text, precision)?)
            } else if shapes_to_paths && !(plain || flatten_transforms || flatten_groups) {
                Some(convert_shapes_svg(&text, precision).map_err(|e| SvgoError(e.to_string()))?)
            } else {
                Some(flatten_svg(&text, precision, flatten_transforms, flatten_groups, shapes_to_paths, plain).map_err(|e| SvgoError(e.to_string()))?)
            }
        }
        "viewbox" | "b" => {
            let mut text = recipe_current_text(&current, input_path)?;
            let precision = recipe_usize(step, &["precision"])?;
            let remove_dimensions = recipe_bool(step, &["removeDimensions", "remove_dimensions"], false);
            if recipe_bool(step, &["fitContent", "fit_content"], false) {
                let padding = recipe_f64(step, &["padding"])?.unwrap_or(0.0);
                text = fit_viewbox_svg(&text, padding, precision, remove_dimensions).map_err(|e| SvgoError(e.to_string()))?;
            } else if let Some(viewbox) = recipe_string(step, &["set", "viewBox", "viewbox"]) {
                text = set_viewbox_svg_core(&text, &viewbox, precision, remove_dimensions)?;
            } else if remove_dimensions {
                return Err("--remove-dimensions requires set/viewBox or fitContent in recipe viewbox step".into());
            }
            let width = recipe_string(step, &["width"]);
            let height = recipe_string(step, &["height"]);
            if width.is_some() || height.is_some() {
                text = resize_svg(&text, width.as_deref(), height.as_deref()).map_err(|e| SvgoError(e.to_string()))?;
            }
            Some(text)
        }
        "path" | "p" => {
            let text = recipe_current_text(&current, input_path)?;
            let select = recipe_string(step, &["select"]).unwrap_or_else(|| "all".to_string());
            let ops = recipe_string_list(step, &["ops", "op"])?;
            let decimals = recipe_usize(step, &["decimals", "precision"])?.unwrap_or(4);
            let minify = recipe_bool(step, &["minify"], false);
            let svgo = recipe_bool(step, &["svgo"], false);
            let svgo_order = recipe_string(step, &["svgoOrder", "svgo_order"]).unwrap_or_else(|| "after".to_string());
            Some(edit_svg_text(&text, &select, &ops, decimals, minify, Some(recipe_opt_options(step)?), &svgo_order, svgo)?)
        }
        "center" | "c" => {
            let options = recipe_center_options(step)?;
            if let Some(path) = recipe_string(step, &["path", "d"]) {
                let (d, stroke_width, ctx) = centerline_path_data_core(&path, options.clone())?;
                Some(build_centerline_output(&d, &options.emit, stroke_width, &options, ctx))
            } else {
                let text = recipe_current_text(&current, input_path)?;
                if contains_svg_markup(&text) && options.svg_paths == "all" {
                    Some(centerline_svg_text_core(&text, options.clone())?)
                } else {
                    let d = find_d_attributes(&text).first().map(|m| m.3.clone()).unwrap_or(text.trim().to_string());
                    let (center_d, stroke_width, ctx) = centerline_path_data_core(&d, options.clone())?;
                    Some(build_centerline_output(&center_d, &options.emit, stroke_width, &options, ctx))
                }
            }
        }
        "opt" | "o" => {
            let text = recipe_current_text(&current, input_path)?;
            Some(optimize_svg_core(&text, recipe_opt_options(step)?)?)
        }
        "validate" | "v" => {
            let text = recipe_current_text(&current, input_path)?;
            let strict = recipe_bool(step, &["strict"], false);
            let result = validate_svg_value(&text, strict);
            let valid = result.get("valid").and_then(Value::as_bool).unwrap_or(false);
            if let Some(map) = report.as_object_mut() {
                map.insert("valid".to_string(), json!(valid));
                map.insert("issues".to_string(), result.get("issues").cloned().unwrap_or_else(|| json!([])));
            }
            if !valid && recipe_bool(step, &["fail", "failOnInvalid", "fail_on_invalid"], true) {
                return Err(SvgoError(format!("recipe validation failed for {}", input_path.display())));
            }
            Some(text)
        }
        "info" | "i" => {
            let text = recipe_current_text(&current, input_path)?;
            if let Some(map) = report.as_object_mut() {
                map.insert("info".to_string(), get_svg_info_value(&text));
            }
            Some(text)
        }
        "measure" | "m" => {
            let text = recipe_current_text(&current, input_path)?;
            let decimals = recipe_usize(step, &["decimals", "precision"])?;
            let error = recipe_f64(step, &["error"])?.unwrap_or(0.01);
            if let Some(map) = report.as_object_mut() {
                map.insert("metrics".to_string(), svg_metrics_value(&text, decimals, error));
            }
            Some(text)
        }
        _ => return Err(SvgoError(format!("unknown recipe command {}", command))),
    };
    Ok((next, report))
}

fn recipe_template(kind: &str) -> Result<Value> {
    match kind {
        "cleanup" | "svg-cleanup" => Ok(json!({
            "name": "svg-cleanup",
            "description": "Sanitize, flatten, fit the viewBox, validate, and optimize SVG files.",
            "outputExtension": ".svg",
            "steps": [
                {"command": "sanitize", "removeExternalRefs": true},
                {"command": "convert", "all": true, "precision": 3},
                {"command": "viewbox", "fitContent": true, "padding": 1, "removeDimensions": true, "precision": 3},
                {"command": "validate", "strict": true},
                {"command": "opt", "multipass": true, "precision": 3}
            ]
        })),
        "centerline-icons" | "png-centerline" => Ok(json!({
            "name": "centerline-icons",
            "description": "Trace palette PNG icons, reconstruct colored centerline strokes, and optimize SVG output.",
            "outputExtension": ".svg",
            "steps": [
                {"command": "trace", "mode": "palette", "palette": ["#143861", "#00b795"], "dropWhite": true, "whiteThreshold": 245, "alphaThreshold": 16, "minArea": 80, "decimals": 1},
                {"command": "center", "svgPaths": "all", "mode": "all", "polyline": true, "bridgeGap": 12, "keepFailed": true, "strokeWidth": "auto", "decimals": 2},
                {"command": "opt", "multipass": true, "precision": 2}
            ]
        })),
        "path-edit" => Ok(json!({
            "name": "path-edit",
            "description": "Apply ordered path operations, then optimize SVG output.",
            "outputExtension": ".svg",
            "steps": [
                {"command": "path", "select": "all", "ops": ["absolute", "optimize:safe"], "decimals": 3, "minify": true},
                {"command": "opt", "multipass": true, "precision": 3}
            ]
        })),
        _ => Err(SvgoError(format!("unknown recipe template kind {}", kind))),
    }
}

fn run_recipe_init_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut output = None;
    let mut kind = "cleanup".to_string();
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--kind" => kind = cursor.value("--kind")?,
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let text = serde_json::to_string_pretty(&recipe_template(&kind)?).unwrap();
    write_file_or_return(text, output)
}

fn collect_recipe_inputs(input: &Path) -> Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    if !input.is_dir() {
        return Err(SvgoError(format!("recipe input does not exist: {}", input.display())));
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(input).map_err(|e| SvgoError(e.to_string()))? {
        let path = entry.map_err(|e| SvgoError(e.to_string()))?.path();
        if path.is_file() {
            let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("").to_ascii_lowercase();
            if ext == "svg" || ext == "png" {
                files.push(path);
            }
        }
    }
    files.sort();
    if files.is_empty() {
        return Err(SvgoError(format!("no SVG or PNG files found in {}", input.display())));
    }
    Ok(files)
}

fn recipe_output_extension(recipe: &Value) -> String {
    recipe_string(recipe, &["outputExtension", "output_extension"])
        .unwrap_or_else(|| ".svg".to_string())
        .trim_start_matches('.')
        .to_string()
}

fn recipe_output_for(input_path: &Path, root_input: &Path, output: Option<&Path>, recipe: &Value, multiple: bool) -> Result<Option<PathBuf>> {
    let Some(output) = output else {
        if multiple {
            return Err("--output is required when recipe input is a directory".into());
        }
        return Ok(None);
    };
    if multiple || root_input.is_dir() || output.is_dir() || output.extension().is_none() {
        let stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| SvgoError(format!("could not derive output name for {}", input_path.display())))?;
        Ok(Some(output.join(format!("{}.{}", stem, recipe_output_extension(recipe)))))
    } else {
        Ok(Some(output.to_path_buf()))
    }
}

fn write_recipe_output(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| SvgoError(e.to_string()))?;
        }
    }
    fs::write(path, if text.ends_with('\n') { text.to_string() } else { format!("{}\n", text) }).map_err(|e| SvgoError(e.to_string()))
}

fn run_recipe_on_file(recipe: &Value, input_path: &Path) -> Result<(String, Value)> {
    let steps = recipe
        .get("steps")
        .and_then(Value::as_array)
        .ok_or_else(|| SvgoError("recipe requires a steps array".to_string()))?;
    if steps.is_empty() {
        return Err("recipe steps array is empty".into());
    }
    let mut current = None;
    let mut step_reports = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let (next, mut report) = apply_recipe_step(current, input_path, step)?;
        if let Some(map) = report.as_object_mut() {
            map.insert("index".to_string(), json!(index));
        }
        current = next;
        step_reports.push(report);
    }
    let output = recipe_current_text(&current, input_path)?;
    let bytes = output.len();
    let report = json!({
        "input": input_path.display().to_string(),
        "steps": step_reports,
        "bytes": bytes
    });
    Ok((output, report))
}

fn run_recipe_run_cli(args: &[String]) -> Result<String> {
    let mut cursor = ArgCursor::new(args);
    let mut recipe_path = None;
    let mut input = None;
    let mut output = None;
    let mut report_path = None;
    let mut compact = false;
    while let Some(arg) = cursor.next() {
        match arg.as_str() {
            "--recipe" | "-r" => recipe_path = Some(cursor.value("--recipe")?),
            "--input" | "-i" => input = Some(cursor.value("--input")?),
            "--output" | "-o" => output = Some(cursor.value("--output")?),
            "--report" => report_path = Some(cursor.value("--report")?),
            "--compact" => compact = true,
            _ => return Err(SvgoError(format!("unknown option {}", arg))),
        }
    }
    let recipe_path = recipe_path.ok_or_else(|| SvgoError("--recipe is required".to_string()))?;
    let input = input.ok_or_else(|| SvgoError("--input is required".to_string()))?;
    let recipe_text = read_file(&recipe_path)?;
    let recipe: Value = serde_json::from_str(&recipe_text).map_err(|e| SvgoError(format!("Invalid recipe JSON: {}", e)))?;
    let input_path = PathBuf::from(input);
    let output_path = output.as_ref().map(|value| PathBuf::from(value.as_str()));
    let inputs = collect_recipe_inputs(&input_path)?;
    let multiple = inputs.len() > 1 || input_path.is_dir();
    let mut reports = Vec::new();
    let mut single_stdout = None;
    for path in inputs {
        let (text, mut item_report) = run_recipe_on_file(&recipe, &path)?;
        let target = recipe_output_for(&path, &input_path, output_path.as_deref(), &recipe, multiple)?;
        if let Some(target) = target {
            write_recipe_output(&target, &text)?;
            if let Some(map) = item_report.as_object_mut() {
                map.insert("output".to_string(), json!(target.display().to_string()));
            }
        } else {
            single_stdout = Some(text);
        }
        reports.push(item_report);
    }
    if let Some(report_path) = report_path {
        let report_text = if compact { serde_json::to_string(&reports).unwrap() } else { serde_json::to_string_pretty(&reports).unwrap() };
        write_recipe_output(Path::new(&report_path), &report_text)?;
    }
    if let Some(text) = single_stdout {
        Ok(text)
    } else if compact {
        Ok(serde_json::to_string(&reports).unwrap())
    } else {
        Ok(serde_json::to_string_pretty(&reports).unwrap())
    }
}

fn run_recipe_cli(args: &[String]) -> Result<String> {
    match args.first().map(String::as_str) {
        Some("init") => run_recipe_init_cli(&args[1..]),
        Some("run") => run_recipe_run_cli(&args[1..]),
        Some(other) if other.starts_with('-') => run_recipe_run_cli(args),
        Some(other) => Err(SvgoError(format!("unknown recipe action {}", other))),
        None => Err("Use `svgo recipe init` or `svgo recipe run`".into()),
    }
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
        Some("recipe") | Some("r") => r#"Usage:
  svgo recipe init [--kind <cleanup|centerline-icons|path-edit>] [--output <FILE>]
  svgo recipe run --recipe <JSON_FILE> --input <FILE_OR_DIR> [--output <FILE_OR_DIR>] [--report <FILE>]
  svgo r ...

Build and run declarative JSON recipes from existing svgo commands.

Actions:
  init                       Emit a starter JSON recipe.
  run                        Apply a recipe to one SVG/PNG file or a directory.

Run options:
  -r, --recipe <FILE>        Recipe JSON file.
  -i, --input <FILE_OR_DIR>  Input SVG/PNG file or directory.
  -o, --output <FILE_OR_DIR> Write output. Required for directory input.
  --report <FILE>            Write per-file step report JSON.
  --compact                  Emit compact report JSON.

Recipe steps use command names such as validate, sanitize, convert, viewbox,
path, trace, trace2, center, opt, info, and measure. Step option names are the
long CLI option names converted to JSON keys, for example fitContent,
removeDimensions, svgPaths, bridgeGap, and multipass.

Examples:
  svgo recipe init --kind cleanup -o cleanup.svgo.json
  svgo recipe run -r cleanup.svgo.json -i icons -o icons-out --report report.json"#.to_string(),
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
  recipe, r        Run declarative SVG conversion recipes.

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
            | "viewbox" | "b" | "convert" | "x" | "plugins" | "l" | "recipe" | "r" => {
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
        "recipe" | "r" => run_recipe_cli(rest).map(|out| (0, out)),
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

