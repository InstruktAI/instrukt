//! Rendering configuration values as YAML that every YAML reader reads
//! back as the same value.
//!
//! The YAML library quotes a string only where a YAML 1.2 reader would take
//! it for another type. YAML 1.1 readers also take `on`, `off`, `yes`, `no`,
//! `y` and `n` (in any common case) for booleans, and `ticks: on` is the
//! string `on`. Such strings are rendered single-quoted.

use serde_yaml_ng::{Mapping, Value};

/// Strings a YAML 1.1 reader takes for booleans when unquoted.
const YAML11_BOOLEANS: [&str; 22] = [
    "y", "Y", "yes", "Yes", "YES", "n", "N", "no", "No", "NO", "on", "On", "ON", "off", "Off",
    "OFF", "true", "True", "TRUE", "false", "False", "FALSE",
];

/// `value` as YAML text.
pub fn render_yaml(value: &Value) -> Result<String, serde_yaml_ng::Error> {
    let plain = serde_yaml_ng::to_string(value)?;
    let marker = unused_marker(&plain);
    let mut quoted = Vec::new();
    let masked = mask(value, &marker, &mut quoted);
    if quoted.is_empty() {
        return Ok(plain);
    }
    let mut text = serde_yaml_ng::to_string(&masked)?;
    // Replace the highest index first, so `marker1` never matches inside
    // `marker10`.
    for (index, word) in quoted.iter().enumerate().rev() {
        text = text.replace(&format!("{marker}{index}"), &format!("'{word}'"));
    }
    Ok(text)
}

/// A plain-scalar prefix that occurs nowhere in `text`.
fn unused_marker(text: &str) -> String {
    let mut marker = String::from("instruktquoted");
    while text.contains(&marker) {
        marker.push('x');
    }
    marker
}

/// `value` with every YAML 1.1 boolean word (key or string) replaced by
/// `<marker><index>`; `quoted[index]` holds the word.
fn mask(value: &Value, marker: &str, quoted: &mut Vec<String>) -> Value {
    match value {
        Value::String(text) => mask_str(text, marker, quoted),
        Value::Mapping(map) => {
            let mut out = Mapping::new();
            for (key, child) in map {
                let key = match key {
                    Value::String(text) => mask_str(text, marker, quoted),
                    other => other.clone(),
                };
                out.insert(key, mask(child, marker, quoted));
            }
            Value::Mapping(out)
        }
        Value::Sequence(seq) => Value::Sequence(
            seq.iter()
                .map(|child| mask(child, marker, quoted))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn mask_str(text: &str, marker: &str, quoted: &mut Vec<String>) -> Value {
    if YAML11_BOOLEANS.contains(&text) {
        quoted.push(text.to_owned());
        Value::String(format!("{marker}{}", quoted.len() - 1))
    } else {
        Value::String(text.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn value() -> impl Strategy<Value = Value> {
        let words: Vec<String> = YAML11_BOOLEANS.iter().map(|w| (*w).to_owned()).collect();
        let text = prop_oneof![
            prop::sample::select(words),
            "[a-z ]{0,6}",
            Just("instruktquoted0".to_owned()),
        ];
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i32>().prop_map(|n| Value::Number(n.into())),
            text.clone().prop_map(Value::String),
        ];
        leaf.prop_recursive(3, 32, 6, move |inner| {
            prop_oneof![
                prop::collection::vec(inner.clone(), 0..5).prop_map(Value::Sequence),
                prop::collection::btree_map(text.clone(), inner, 0..5).prop_map(|map| {
                    Value::Mapping(
                        map.into_iter()
                            .map(|(k, v)| (Value::String(k), v))
                            .collect(),
                    )
                }),
            ]
        })
    }

    /// How many leaves of `value` are booleans.
    fn booleans(value: &Value) -> usize {
        match value {
            Value::Bool(_) => 1,
            Value::Mapping(map) => map.values().map(booleans).sum(),
            Value::Sequence(seq) => seq.iter().map(booleans).sum(),
            _ => 0,
        }
    }

    /// How many plain (unquoted) scalars of block-style YAML `text` a YAML
    /// 1.1 reader takes for booleans: keys, values and sequence items.
    fn yaml11_boolean_scalars(text: &str) -> usize {
        let mut scalars = Vec::new();
        for line in text.lines() {
            let mut rest = line.trim_start();
            while let Some(item) = rest.strip_prefix("- ") {
                rest = item.trim_start();
            }
            match rest.split_once(": ") {
                Some((key, value)) => scalars.extend([key, value]),
                None => scalars.push(rest.strip_suffix(':').unwrap_or(rest)),
            }
        }
        scalars
            .into_iter()
            .filter(|scalar| YAML11_BOOLEANS.contains(&scalar.trim()))
            .count()
    }

    proptest! {
        #[test]
        fn rendered_yaml_reads_back_as_the_same_value(value in value()) {
            let text = render_yaml(&value).unwrap();
            let read: Value = serde_yaml_ng::from_str(&text).unwrap();
            prop_assert_eq!(read, value);
        }

        #[test]
        fn only_booleans_render_as_yaml11_boolean_tokens(value in value()) {
            let text = render_yaml(&value).unwrap();
            prop_assert_eq!(yaml11_boolean_scalars(&text), booleans(&value), "{}", text);
        }
    }

    #[test]
    fn no_string_renders_as_a_yaml11_boolean_token() {
        let mut map = Mapping::new();
        for word in YAML11_BOOLEANS {
            map.insert(
                Value::String(word.to_owned()),
                Value::Sequence(vec![Value::String(word.to_owned()), Value::Bool(true)]),
            );
        }
        let value = Value::Mapping(map);
        let text = render_yaml(&value).unwrap();
        assert_eq!(yaml11_boolean_scalars(&text), YAML11_BOOLEANS.len());
        let plain = serde_yaml_ng::to_string(&value).unwrap();
        assert!(
            yaml11_boolean_scalars(&plain) > YAML11_BOOLEANS.len(),
            "the check sees what the plain serializer leaves bare"
        );
    }
}
