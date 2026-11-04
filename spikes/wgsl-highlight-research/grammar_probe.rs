use serde_json::{json, Value};
use std::{fs, time::Instant};
use tree_sitter::{InputEdit, Node, Parser, Point, Tree};

fn anomalies(node: Node<'_>, code: &str, result: &mut Vec<Value>, boundaries: &mut bool) {
    *boundaries &=
        code.is_char_boundary(node.start_byte()) && code.is_char_boundary(node.end_byte());
    if node.is_error() || node.is_missing() {
        result.push(json!({"kind":node.kind(),"missing":node.is_missing(),"start":node.start_byte(),"end":node.end_byte(),"text":code.get(node.byte_range()).unwrap_or("<bad UTF8>")}));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        anomalies(child, code, result, boundaries);
    }
}
fn median_us(samples: &mut [u128]) -> f64 {
    samples.sort_unstable();
    samples[samples.len() / 2] as f64 / 1000.0
}
fn bench(parser: &mut Parser, code: &str) -> Value {
    let original = parser.parse(code, None).unwrap();
    let edited_code = format!("// research\n{code}");
    let insertion = "// research\n";
    let mut edit_tree = original.clone();
    edit_tree.edit(&InputEdit {
        start_byte: 0,
        old_end_byte: 0,
        new_end_byte: insertion.len(),
        start_position: Point::new(0, 0),
        old_end_position: Point::new(0, 0),
        new_end_position: Point::new(1, 0),
    });
    let mut fresh = vec![];
    let mut incremental = vec![];
    let mut incremental_matches = true;
    for _ in 0..101 {
        let start = Instant::now();
        let tree = parser.parse(&edited_code, None).unwrap();
        fresh.push(start.elapsed().as_nanos());
        let start = Instant::now();
        let incr = parser.parse(&edited_code, Some(&edit_tree)).unwrap();
        incremental.push(start.elapsed().as_nanos());
        incremental_matches &= tree.root_node().to_sexp() == incr.root_node().to_sexp();
    }
    json!({"bytes":code.len(),"iterations":101,"fresh_median_us":median_us(&mut fresh),"incremental_median_us":median_us(&mut incremental),"incremental_matches_fresh":incremental_matches})
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input: Value = serde_json::from_str(&fs::read_to_string(&args[1]).unwrap()).unwrap();
    let mut parser = Parser::new();
    #[cfg(not(feature = "maintained"))]
    {
        parser.set_language(tree_sitter_wgsl::language()).unwrap();
    }
    #[cfg(feature = "maintained")]
    {
        parser
            .set_language(&tree_sitter_wgsl_bevy::LANGUAGE.into())
            .unwrap();
    }
    let mut results = vec![];
    let mut catalog_successes = 0;
    for case in input["cases"].as_array().unwrap() {
        let code = case["source"].as_str().unwrap();
        let tree: Tree = parser.parse(code, None).unwrap();
        let mut errors = vec![];
        let mut boundaries = true;
        anomalies(tree.root_node(), code, &mut errors, &mut boundaries);
        let good = !tree.root_node().has_error();
        if case["catalog"].as_bool().unwrap_or(false) && good {
            catalog_successes += 1;
        }
        results.push(json!({"label":case["label"],"catalog":case["catalog"],"bytes":code.len(),"has_error":!good,"utf8_boundaries_valid":boundaries,"anomaly_count":errors.len(),"anomalies":errors.into_iter().take(6).collect::<Vec<_>>(),"tree":if case["catalog"].as_bool().unwrap_or(false){Value::Null}else{json!(tree.root_node().to_sexp())}}));
    }
    let mut benchmarks = vec![];
    let base = "fn research() -> vec4f { return vec4f(0.1, 0.2, 0.3, 1.0); }\n";
    for lines in [16, 160, 1600] {
        benchmarks.push(bench(&mut parser, &base.repeat(lines)));
    }
    let output = json!({"candidate":if cfg!(feature="maintained"){"tree-sitter-wgsl-bevy 0.1.4 / tree-sitter 0.25.10"}else{"tree-sitter-wgsl 0.0.6 / tree-sitter 0.20.10"},"catalog_successes":catalog_successes,"results":results,"parser_only_benchmarks":benchmarks});
    fs::write(&args[2], serde_json::to_string_pretty(&output).unwrap()).unwrap();
    println!(
        "{}",
        json!({"candidate":output["candidate"],"catalog_successes":catalog_successes,"cases":results.len(),"parser_only_benchmarks":output["parser_only_benchmarks"]})
    );
}
