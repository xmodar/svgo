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
use std::path::{Path, PathBuf};
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

include!("core.rs");
include!("pathdata.rs");
include!("geometry.rs");
include!("measure.rs");
include!("svg.rs");
include!("trace.rs");
include!("centerline.rs");
include!("cli.rs");
include!("python.rs");
