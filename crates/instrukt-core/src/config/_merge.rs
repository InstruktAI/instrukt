//! Combining documents: the merge patch applied to a layer, and the overlay
//! of one layer on another.

use serde_yaml_ng::Value;

/// Applies `patch` to `target` as a merge patch: mappings merge key by key,
/// a null removes the key, any other value replaces.
pub(crate) fn merge_patch(target: &mut Value, patch: &Value) {
    let Value::Mapping(patch_map) = patch else {
        *target = patch.clone();
        return;
    };
    if !target.is_mapping() {
        *target = Value::Mapping(serde_yaml_ng::Mapping::new());
    }
    let Value::Mapping(target_map) = target else {
        return;
    };
    for (key, value) in patch_map {
        if value.is_null() {
            target_map.remove(key);
            continue;
        }
        match target_map.get_mut(key) {
            Some(existing) => merge_patch(existing, value),
            None => {
                let mut fresh = Value::Null;
                merge_patch(&mut fresh, value);
                target_map.insert(key.clone(), fresh);
            }
        }
    }
}

/// Applies `upper` over `base` key by key: mappings merge recursively, any
/// other value of `upper` (null included) replaces.
pub(crate) fn overlay(base: &mut Value, upper: &Value) {
    match (base, upper) {
        (Value::Mapping(base_map), Value::Mapping(upper_map)) => {
            for (key, value) in upper_map {
                match base_map.get_mut(key) {
                    Some(existing) => overlay(existing, value),
                    None => {
                        base_map.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (base, upper) => *base = upper.clone(),
    }
}

/// Removes from `upper` every entry whose value equals `base`, keeping
/// `overlay(base, upper)` identical: mappings recurse key by key and drop when
/// emptied; any other value drops when equal to `base` whole; keys absent from
/// `base` stay intact.
pub(crate) fn prune(upper: &mut Value, base: &Value) {
    let (Value::Mapping(upper_map), Value::Mapping(base_map)) = (upper, base) else {
        return;
    };
    upper_map.retain(|key, upper_val| {
        let Some(base_val) = base_map.get(key) else {
            return true;
        };
        if upper_val.is_mapping() && base_val.is_mapping() {
            prune(upper_val, base_val);
            !upper_val
                .as_mapping()
                .is_some_and(serde_yaml_ng::Mapping::is_empty)
        } else {
            *upper_val != *base_val
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    #[test]
    fn mappings_merge_nulls_remove_and_other_values_replace() {
        let mut doc = yaml("a: 1\nb: {c: 2, d: 3}\nl: [1, 2]\n");
        merge_patch(
            &mut doc,
            &yaml("b: {c: ~, e: 4}\nl: [9]\na: ~\nn: {x: ~, y: 1}\n"),
        );
        assert_eq!(doc, yaml("b: {d: 3, e: 4}\nl: [9]\nn: {y: 1}\n"));
    }

    #[test]
    fn overlay_keeps_base_keys_and_replaces_scalars() {
        let mut base = yaml("modules: {todos: true}\nticks: on\n");
        overlay(&mut base, &yaml("modules: {blueprint: true}\nticks: off\n"));
        assert_eq!(
            base,
            yaml("modules: {todos: true, blueprint: true}\nticks: off\n")
        );
    }

    #[test]
    fn prune_recurses_into_mappings_drops_equal_values_and_keeps_unknown_keys() {
        let base = yaml(
            "nested: {same: 1, diff: 2}\nemptied: {x: 1, y: 2}\nsame_list: [a, b]\ndiff_list: [a, b]\n",
        );
        let mut upper = yaml(
            "nested: {same: 1, diff: 9, extra: 3}\nemptied: {x: 1, y: 2}\nsame_list: [a, b]\ndiff_list: [a, c]\nunknown: {k: v}\nunknown_empty: {}\n",
        );
        prune(&mut upper, &base);
        assert_eq!(
            upper,
            yaml(
                "nested: {diff: 9, extra: 3}\ndiff_list: [a, c]\nunknown: {k: v}\nunknown_empty: {}\n"
            )
        );
    }

    fn value() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::Bool),
            any::<i32>().prop_map(|n| Value::Number(n.into())),
            "[a-z]{0,4}".prop_map(Value::String),
        ];
        leaf.prop_recursive(3, 24, 4, |inner| {
            prop::collection::btree_map("[a-c]", inner, 0..4).prop_map(|map| {
                Value::Mapping(
                    map.into_iter()
                        .map(|(k, v)| (Value::String(k), v))
                        .collect(),
                )
            })
        })
    }

    fn has_null(value: &Value) -> bool {
        match value {
            Value::Null => true,
            Value::Mapping(map) => map.values().any(has_null),
            _ => false,
        }
    }

    proptest! {
        #[test]
        fn applying_a_patch_twice_equals_applying_it_once(target in value(), patch in value()) {
            let mut once = target.clone();
            merge_patch(&mut once, &patch);
            let mut twice = once.clone();
            merge_patch(&mut twice, &patch);
            prop_assert_eq!(once, twice);
        }

        #[test]
        fn a_mapping_patch_leaves_no_null_it_introduced(target in value(), patch in value()) {
            prop_assume!(patch.is_mapping() && !has_null(&target));
            let mut doc = target;
            merge_patch(&mut doc, &patch);
            prop_assert!(!has_null(&doc));
        }

        #[test]
        fn overlaying_the_pruned_upper_equals_overlaying_the_upper(base in value(), upper in value()) {
            let mut direct = base.clone();
            overlay(&mut direct, &upper);
            let mut pruned = upper;
            prune(&mut pruned, &base);
            let mut via_pruned = base;
            overlay(&mut via_pruned, &pruned);
            prop_assert_eq!(via_pruned, direct);
        }

        #[test]
        fn pruning_twice_equals_pruning_once(base in value(), upper in value()) {
            let mut once = upper;
            prune(&mut once, &base);
            let mut twice = once.clone();
            prune(&mut twice, &base);
            prop_assert_eq!(once, twice);
        }
    }
}
