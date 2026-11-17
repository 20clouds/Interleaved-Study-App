// Robust JSON extraction from model output: strip code fences, take the outermost array/object.
// Plus the safe field readers and character caps every validator applies to what comes out.
use crate::rx;
use crate::util::take_chars;
use serde_json::Value;

/// The model's answer was not the JSON asked for; one repair request may follow.
#[derive(Debug, Clone)]
pub struct InvalidJson(pub String);

pub mod cap {
    pub const NAME: usize = 120;
    pub const TERM: usize = 80;
    pub const SUMMARY: usize = 400;
    pub const QUIZ: usize = 500;
    pub const FEEDBACK: usize = 200;
    pub const COURSE: usize = 120;
    pub const NOTES: usize = 500;
    pub const ANSWER: usize = 2000;
    pub const QUOTE: usize = 200;
}

fn js_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

/// Model output as trimmed, capped text. Only strings, numbers and booleans count.
pub fn str_of(v: Option<&Value>, max: usize) -> String {
    let s = match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => js_number(n.as_f64().unwrap_or(0.0)),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    };
    take_chars(s.trim(), max)
}

/// Model output as a number: numbers and numeric strings only, else NaN.
pub fn num_of(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().parse::<f64>().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

/// Up to `max` non-empty strings of at most `chars` characters each.
pub fn str_list(v: Option<&Value>, max: usize, chars: usize) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().map(|x| str_of(Some(x), chars)).filter(|s| !s.is_empty()).take(max).collect(),
        _ => vec![],
    }
}

pub fn snippet(v: &Value, n: usize) -> String {
    take_chars(&v.to_string(), n)
}

pub fn extract_json(raw: &str) -> Result<Value, InvalidJson> {
    let s = raw.trim();
    let s = rx!(r"(?m)^```(?:json|JSON)?\s*").replace(s, "");
    let s = rx!(r"(?m)```\s*$").replace(&s, "");
    let s = s.trim();
    if let Ok(v) = serde_json::from_str::<Value>(s) { return Ok(v) }
    let mut cands: Vec<(usize, char)> = Vec::new();
    if let Some(i) = s.find('{') { cands.push((i, '}')); }
    if let Some(i) = s.find('[') { cands.push((i, ']')); }
    cands.sort_by_key(|c| c.0);
    for (start, close) in cands {
        if let Some(end) = s.rfind(close) {
            if end > start {
                let sub = &s[start..=end];
                if let Ok(v) = serde_json::from_str::<Value>(sub) { return Ok(v) }
                let fixed = rx!(r",\s*([}\]])").replace_all(sub, "$1");
                if let Ok(v) = serde_json::from_str::<Value>(&fixed) { return Ok(v) }
            }
        }
    }
    Err(InvalidJson("Model output is not valid JSON".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_from_fences_and_prose() {
        assert_eq!(extract_json("```json\n{\"a\":1}\n```").unwrap(), json!({"a":1}));
        assert_eq!(extract_json("Here you go: {\"a\":[1,2,],} thanks").unwrap(), json!({"a":[1,2]}));
        assert_eq!(extract_json("[1,2]").unwrap(), json!([1, 2]));
        assert!(extract_json("no json here").is_err());
    }

    #[test]
    fn readers_are_strict() {
        assert_eq!(str_of(Some(&json!("  hi  ")), 10), "hi");
        assert_eq!(str_of(Some(&json!(3)), 10), "3");
        assert_eq!(str_of(Some(&json!({"x":1})), 10), "");
        assert!(num_of(Some(&json!("abc"))).is_nan());
        assert_eq!(num_of(Some(&json!("2"))), 2.0);
        assert_eq!(str_list(Some(&json!(["a", "", 2, null, "b"])), 2, 5), vec!["a", "2"]);
    }
}
