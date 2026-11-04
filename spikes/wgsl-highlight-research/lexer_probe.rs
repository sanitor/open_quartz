use logos::Logos;
use serde_json::{json, Value};
use std::{fs, time::Instant};
use wgsl_parse::lexer::Token;
fn inspect(code: &str, detail: bool) -> Value {
    let mut errors = vec![];
    let mut comments = vec![];
    let mut tokens = vec![];
    let mut boundaries = true;
    let mut count = 0;
    for (token, range) in Token::lexer(code).spanned() {
        boundaries &= code.is_char_boundary(range.start) && code.is_char_boundary(range.end);
        match token {
            Ok(token)=>{
                count+=1;
                if matches!(token,Token::LineComment|Token::BlockComment){comments.push(json!({"kind":format!("{token:?}"),"start":range.start,"end":range.end,"text":&code[range.clone()]}));}
                if detail {tokens.push(json!({"kind":format!("{token:?}"),"start":range.start,"end":range.end,"text":&code[range.clone()]}));}
            },
            Err(error)=>errors.push(json!({"error":format!("{error:?}"),"start":range.start,"end":range.end,"text":&code[range]})),
        }
    }
    json!({"token_count":count,"utf8_boundaries_valid":boundaries,"errors":errors,"comments":comments,"tokens":tokens})
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let input: Value = serde_json::from_str(&fs::read_to_string(&args[1]).unwrap()).unwrap();
    let mut results = vec![];
    let mut successes = 0;
    for case in input["cases"].as_array().unwrap() {
        let code = case["source"].as_str().unwrap();
        let catalog = case["catalog"].as_bool().unwrap();
        let mut out = inspect(code, !catalog);
        if catalog && out["errors"].as_array().unwrap().is_empty() {
            successes += 1;
        }
        out["label"] = case["label"].clone();
        out["catalog"] = json!(catalog);
        if !catalog {
            out["naga_parse_error"] = naga::front::wgsl::parse_str(code)
                .err()
                .map(|e| json!(e.emit_to_string(code)))
                .unwrap_or(Value::Null);
        }
        results.push(out);
    }
    let mut benches = vec![];
    let base = "fn research() -> vec4f { return vec4f(0.1, 0.2, 0.3, 1.0); }\n";
    for lines in [16, 160, 1600] {
        let code = base.repeat(lines);
        let mut samples = vec![];
        let mut count = 0;
        for _ in 0..101 {
            let now = Instant::now();
            let tokens = Token::lexer(std::hint::black_box(&code))
                .spanned()
                .collect::<Vec<_>>();
            count = std::hint::black_box(tokens.len());
            samples.push(now.elapsed().as_nanos());
        }
        samples.sort_unstable();
        benches.push(json!({"bytes":code.len(),"tokens":count,"iterations":101,"raw_lexer_median_us":samples[50] as f64/1000.0}));
    }
    let out = json!({"candidate":std::env::var("WGSL_RESEARCH_CANDIDATE").unwrap_or_else(|_| "raw WGSL lexer".into()),"catalog_lexical_successes":successes,"results":results,"lexer_only_benchmarks":benches});
    fs::write(&args[2], serde_json::to_string_pretty(&out).unwrap()).unwrap();
    println!(
        "{}",
        json!({"catalog_lexical_successes":successes,"lexer_only_benchmarks":out["lexer_only_benchmarks"]})
    );
}
